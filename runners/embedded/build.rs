use std::process::Command;
use std::str;
use std::{env, error, fs::File, io::Write, path::Path};

#[derive(serde::Deserialize)]
struct Config {
    parameters: Parameters,
    identifier: Identifier,
    #[allow(dead_code)]
    build: Build,
}

#[derive(serde::Deserialize)]
struct Parameters {
    flash_origin: u32,
    #[serde(default)]
    flash_end: Option<u32>,
    filesystem_boundary: u32,
    filesystem_end: u32,
}

#[derive(serde::Deserialize)]
struct Identifier {
    usb_id_vendor: u16,
    usb_id_product: u16,
    usb_manufacturer: String,
    usb_product: String,
    ccid_issuer: String,
}

#[derive(serde::Deserialize)]
struct Build {
    #[allow(dead_code)]
    build_profile: String,
    #[allow(dead_code)]
    board: String,
}

#[derive(Eq, PartialEq)]
enum SocType {
    Lpc55,
    Nrf52840,
}

macro_rules! add_build_variable {
    ($file:expr, $name:literal, u8) => {
        let value = env!($name);
        let value: u8 = str::parse(value).expect("Version components must be able to fit in a u8.");
        writeln!($file, "pub const {}: u8 = {};", $name, value)
            .expect("Could not write build_constants.rs file");
    };

    ($file:expr, $name:literal, $value:expr, u16) => {
        writeln!($file, "pub const {}: u16 = {};", $name, $value)
            .expect("Could not write build_constants.rs file");
    };

    ($file:expr, $name:literal, $value:expr, u32) => {
        writeln!($file, "pub const {}: u32 = {};", $name, $value)
            .expect("Could not write build_constants.rs file");
    };

    ($file:expr, $name:literal, $value:expr, usize) => {
        writeln!($file, "pub const {}: usize = 0x{:x};", $name, $value)
            .expect("Could not write build_constants.rs file");
    };

    ($file:expr, $name:literal, $value:expr, [u8; 13]) => {
        writeln!($file, "pub const {}: [u8; 13] = {:?};", $name, $value)
            .expect("Could not write build_constants.rs file");
    };

    ($file:expr, $name:literal, $value:expr) => {
        writeln!($file, "pub const {}: &str = \"{}\";", $name, $value)
            .expect("Could not write build_constants.rs file");
    };
}

fn check_build_triplet() -> SocType {
    let target = env::var("TARGET").expect("$TARGET unset");
    let soc_is_lpc55 = env::var_os("CARGO_FEATURE_SOC_LPC55").is_some();
    let soc_is_nrf52840 = env::var_os("CARGO_FEATURE_SOC_NRF52840").is_some();

    if soc_is_lpc55 && !soc_is_nrf52840 {
        if target != "thumbv8m.main-none-eabi" {
            panic!(
                "Wrong build triplet for LPC55, expecting thumbv8m.main-none-eabi, got {}",
                target
            );
        }
        SocType::Lpc55
    } else if soc_is_nrf52840 && !soc_is_lpc55 {
        if target != "thumbv7em-none-eabihf" {
            panic!(
                "Wrong build triplet for NRF52840, expecting thumbv7em-none-eabihf, got {}",
                target
            );
        }
        SocType::Nrf52840
    } else {
        panic!("Multiple or no SOC features set.");
    }
}

fn generate_memory_x(outpath: &Path, template: &str, config: &Config) {
    let buildrs_caveat = r#"/* DO NOT EDIT THIS FILE */
/* This file was generated by build.rs */
"#;

    let template = std::fs::read_to_string(template).expect("cannot read memory.x template file");

    let flash_end = config
        .parameters
        .flash_end
        .unwrap_or(config.parameters.filesystem_boundary);
    let flash_len = flash_end - config.parameters.flash_origin;
    assert!(
        flash_len % 1024 == 0,
        "Flash length must be a multiple of 1024"
    );
    let template = template.replace("##FLASH_LENGTH##", &format!("{}", flash_len >> 10));

    let fs_len = config.parameters.filesystem_end - config.parameters.filesystem_boundary;
    assert!(
        fs_len % 1024 == 0,
        "Flash length must be a multiple of 1024"
    );
    let template = template.replace("##FS_LENGTH##", &format!("{}", fs_len >> 10));

    let template = template.replace(
        "##FS_BASE##",
        &format!("{:x}", config.parameters.filesystem_boundary),
    );
    let template = template.replace(
        "##FLASH_BASE##",
        &format!("{:x}", config.parameters.flash_origin),
    );

    std::fs::write(outpath, [buildrs_caveat, &template].join("")).expect("cannot write memory.x");
}

fn main() -> Result<(), Box<dyn error::Error>> {
    let out_dir = env::var("OUT_DIR").expect("$OUT_DIR unset");

    let config_fn = "cfg.toml";

    println!("cargo:rerun-if-changed={}", config_fn);

    let config = std::fs::read_to_string(&config_fn)
        .expect(&format!("failed reading profile: {}", config_fn)[..]);

    let config: Config = toml::from_str(&config).expect("failed parsing toml configuration");

    // @todo: add profile 'platform' items and cross-check them here ...
    let soc_type = check_build_triplet();

    if config.parameters.filesystem_boundary & 0x3ff != 0 {
        panic!("filesystem boundary is not a multiple of the flash block size (1KB)");
    }

    // open and prepare 'build_constants.rs' output
    let dest_path = Path::new(&out_dir).join("build_constants.rs");
    let mut f = File::create(&dest_path).expect("Could not create file");

    let hash_long_cmd = Command::new("git")
        .args(&["rev-parse", "HEAD"])
        .output()
        .unwrap()
        .stdout;
    let hash_short_cmd = Command::new("git")
        .args(&["rev-parse", "--short", "HEAD"])
        .output()
        .unwrap()
        .stdout;

    let hash_long = str::from_utf8(&hash_long_cmd).unwrap().trim();
    let hash_short = str::from_utf8(&hash_short_cmd).unwrap().trim();

    // write 'build_constants.rs' header
    writeln!(&mut f, "pub mod build_constants {{").expect("Could not write build_constants.rs.");

    add_build_variable!(&mut f, "CARGO_PKG_HASH", hash_long);
    add_build_variable!(&mut f, "CARGO_PKG_HASH_SHORT", hash_short);

    // USB Identifiers
    add_build_variable!(
        &mut f,
        "USB_MANUFACTURER",
        config.identifier.usb_manufacturer
    );
    add_build_variable!(&mut f, "USB_PRODUCT", config.identifier.usb_product);
    add_build_variable!(
        &mut f,
        "USB_ID_VENDOR",
        config.identifier.usb_id_vendor,
        u16
    );
    add_build_variable!(
        &mut f,
        "USB_ID_PRODUCT",
        config.identifier.usb_id_product,
        u16
    );

    // convert ccid_issuer to bytes
    let mut ccid_bytes: [u8; 13] = [0u8; 13];
    let raw_issuer = config.identifier.ccid_issuer.as_bytes();
    ccid_bytes[..raw_issuer.len()].clone_from_slice(raw_issuer);
    add_build_variable!(&mut f, "CCID_ISSUER", ccid_bytes, [u8; 13]);

    add_build_variable!(
        &mut f,
        "CONFIG_FILESYSTEM_BOUNDARY",
        config.parameters.filesystem_boundary,
        usize
    );
    add_build_variable!(
        &mut f,
        "CONFIG_FILESYSTEM_END",
        config.parameters.filesystem_end,
        usize
    );
    add_build_variable!(
        &mut f,
        "CONFIG_FLASH_BASE",
        config.parameters.flash_origin,
        usize
    );
    add_build_variable!(
        &mut f,
        "CONFIG_FLASH_END",
        config
            .parameters
            .flash_end
            .unwrap_or(config.parameters.filesystem_boundary),
        usize
    );

    writeln!(&mut f, "}}").expect("Could not write build_constants.rs.");

    // @todo: move this decision into 'profile.cfg'
    let (memory_x_infix, template_file) = match soc_type {
        SocType::Lpc55 => ("ld/lpc55", "ld/lpc55-memory-template.x"),
        SocType::Nrf52840 => ("ld/nrf52", "ld/nrf52-memory-template.x"),
    };

    println!("cargo:rerun-if-changed={}", template_file);
    println!("cargo:rerun-if-changed={}", template_file);

    let memory_x_dir =
        Path::new(&env::var("CARGO_MANIFEST_DIR").expect("$CARGO_MANIFEST_DIR not set"))
            .join(&memory_x_infix);
    std::fs::create_dir(&memory_x_dir).ok();
    let memory_x = memory_x_dir.join("custom_memory.x");

    generate_memory_x(&memory_x, template_file, &config);

    println!("cargo:rustc-link-search={}/ld", env!("CARGO_MANIFEST_DIR"));
    println!(
        "cargo:rustc-link-search={}/{}",
        env!("CARGO_MANIFEST_DIR"),
        memory_x_infix
    );

    let lockfile =
        cargo_lock::Lockfile::load(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock"))?;
    let pkg_cortex_m_rt = lockfile
        .packages
        .iter()
        .find(|p| p.name.as_str() == "cortex-m-rt");

    if let Some(p) = pkg_cortex_m_rt {
        println!("cargo:rustc-link-arg=-Tcortex-m-rt_{}_link.x", p.version);
    }

    Ok(())
}
