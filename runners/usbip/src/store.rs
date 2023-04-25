// for store!
#![allow(clippy::too_many_arguments)]

use std::{
    fs::{File, OpenOptions},
    io::{Read as _, Seek as _, SeekFrom, Write as _},
    marker::PhantomData,
    path::PathBuf,
};

use littlefs2::{
    const_ram_storage,
    consts::{U1, U512, U8},
    fs::{Allocation, Filesystem},
};
use trussed::{
    store,
    types::{LfsResult, LfsStorage},
    virt::StoreProvider,
};

const STORAGE_SIZE: usize = 512 * 128;

static mut INTERNAL_STORAGE: Option<InternalStorage> = None;
static mut INTERNAL_FS_ALLOC: Option<Allocation<InternalStorage>> = None;
static mut INTERNAL_FS: Option<Filesystem<InternalStorage>> = None;

static mut EXTERNAL_STORAGE: Option<ExternalStorage> = None;
static mut EXTERNAL_FS_ALLOC: Option<Allocation<ExternalStorage>> = None;
static mut EXTERNAL_FS: Option<Filesystem<ExternalStorage>> = None;

static mut VOLATILE_STORAGE: Option<VolatileStorage> = None;
static mut VOLATILE_FS_ALLOC: Option<Allocation<VolatileStorage>> = None;
static mut VOLATILE_FS: Option<Filesystem<VolatileStorage>> = None;

const_ram_storage!(InternalRamStorage, STORAGE_SIZE);
// Modelled after the actual external RAM, see src/flash.rs in the embedded runner
const_ram_storage!(
    name=ExternalRamStorage,
    trait=LfsStorage,
    erase_value=0xff,
    read_size=4,
    write_size=256,
    cache_size_ty=U512,
    block_size=4096,
    block_count=0x2_0000 / 4096,
    lookahead_size_ty=U1,
    filename_max_plus_one_ty=U256,
    path_max_plus_one_ty=U256,
    result=LfsResult,
);
const_ram_storage!(VolatileStorage, STORAGE_SIZE);

// TODO: use 256 -- would cause a panic because formatting fails
type InternalStorage = FilesystemOrRamStorage<InternalRamStorage>;
type ExternalStorage = FilesystemOrRamStorage<ExternalRamStorage>;

pub struct FilesystemStorage<S: LfsStorage> {
    path: PathBuf,
    format: bool,
    _storage: PhantomData<S>,
}

impl<S: LfsStorage> FilesystemStorage<S> {
    fn new(path: PathBuf) -> Self {
        let len = u64::try_from(S::BLOCK_SIZE * S::BLOCK_COUNT).unwrap();
        let format = if let Ok(file) = File::open(&path) {
            assert_eq!(file.metadata().unwrap().len(), len);
            false
        } else {
            let file = File::create(&path).expect("failed to create storage file");
            file.set_len(len).expect("failed to set storage file len");
            true
        };
        Self {
            path,
            format,
            _storage: Default::default(),
        }
    }
}

impl<S: LfsStorage> LfsStorage for FilesystemStorage<S> {
    const READ_SIZE: usize = S::READ_SIZE;
    const WRITE_SIZE: usize = S::WRITE_SIZE;
    const BLOCK_SIZE: usize = S::BLOCK_SIZE;

    const BLOCK_COUNT: usize = S::BLOCK_COUNT;
    const BLOCK_CYCLES: isize = S::BLOCK_CYCLES;

    type CACHE_SIZE = U512;
    type LOOKAHEAD_SIZE = U8;

    fn read(&mut self, offset: usize, buffer: &mut [u8]) -> LfsResult<usize> {
        let mut file = File::open(&self.path).unwrap();
        file.seek(SeekFrom::Start(offset as _)).unwrap();
        let bytes_read = file.read(buffer).unwrap();
        assert!(bytes_read <= buffer.len());
        Ok(bytes_read as _)
    }

    fn write(&mut self, offset: usize, data: &[u8]) -> LfsResult<usize> {
        if offset + data.len() > STORAGE_SIZE {
            return Err(littlefs2::io::Error::NoSpace);
        }
        let mut file = OpenOptions::new().write(true).open(&self.path).unwrap();
        file.seek(SeekFrom::Start(offset as _)).unwrap();
        let bytes_written = file.write(data).unwrap();
        assert_eq!(bytes_written, data.len());
        file.flush().unwrap();
        Ok(bytes_written)
    }

    fn erase(&mut self, offset: usize, len: usize) -> LfsResult<usize> {
        if offset + len > STORAGE_SIZE {
            return Err(littlefs2::io::Error::NoSpace);
        }
        let mut file = OpenOptions::new().write(true).open(&self.path).unwrap();
        file.seek(SeekFrom::Start(offset as _)).unwrap();
        let zero_block = vec![0xFFu8; S::BLOCK_SIZE];
        for _ in 0..(len / Self::BLOCK_SIZE) {
            let bytes_written = file.write(&zero_block).unwrap();
            assert_eq!(bytes_written, Self::BLOCK_SIZE);
        }
        file.flush().unwrap();
        Ok(len)
    }
}

pub enum FilesystemOrRamStorage<S: LfsStorage> {
    Filesystem(FilesystemStorage<S>),
    Ram(S),
}

impl<S: LfsStorage + Default> FilesystemOrRamStorage<S> {
    fn new(path: Option<PathBuf>) -> Self {
        path.map(Self::filesystem).unwrap_or_default()
    }

    fn filesystem(path: PathBuf) -> Self {
        Self::Filesystem(FilesystemStorage::new(path))
    }

    fn format(&self) -> bool {
        match self {
            Self::Filesystem(fs) => fs.format,
            Self::Ram(_) => true,
        }
    }
}

impl<S: LfsStorage + Default> Default for FilesystemOrRamStorage<S> {
    fn default() -> Self {
        Self::Ram(Default::default())
    }
}

impl<S: LfsStorage> LfsStorage for FilesystemOrRamStorage<S> {
    const READ_SIZE: usize = S::READ_SIZE;
    const WRITE_SIZE: usize = S::WRITE_SIZE;
    const BLOCK_SIZE: usize = S::BLOCK_SIZE;

    const BLOCK_COUNT: usize = S::BLOCK_COUNT;
    const BLOCK_CYCLES: isize = S::BLOCK_CYCLES;

    type CACHE_SIZE = U512;
    type LOOKAHEAD_SIZE = U8;

    fn read(&mut self, offset: usize, buffer: &mut [u8]) -> LfsResult<usize> {
        match self {
            Self::Filesystem(storage) => storage.read(offset, buffer),
            Self::Ram(storage) => storage.read(offset, buffer),
        }
    }

    fn write(&mut self, offset: usize, data: &[u8]) -> LfsResult<usize> {
        match self {
            Self::Filesystem(storage) => storage.write(offset, data),
            Self::Ram(storage) => storage.write(offset, data),
        }
    }

    fn erase(&mut self, offset: usize, len: usize) -> LfsResult<usize> {
        match self {
            Self::Filesystem(storage) => storage.erase(offset, len),
            Self::Ram(storage) => storage.erase(offset, len),
        }
    }
}

store!(
    Store,
    Internal: InternalStorage,
    External: ExternalStorage,
    Volatile: VolatileStorage
);

#[derive(Clone, Debug, Default)]
pub struct FilesystemOrRam {
    ifs: Option<PathBuf>,
    efs: Option<PathBuf>,
}

impl FilesystemOrRam {
    pub fn new(ifs: Option<PathBuf>, efs: Option<PathBuf>) -> Self {
        Self { ifs, efs }
    }
}

impl StoreProvider for FilesystemOrRam {
    type Store = Store;

    unsafe fn ifs() -> &'static mut InternalStorage {
        INTERNAL_STORAGE.as_mut().expect("ifs not initialized")
    }

    unsafe fn store() -> Self::Store {
        Self::Store { __: PhantomData }
    }

    unsafe fn reset(&self) {
        let ifs = reset_internal(InternalStorage::new(self.ifs.clone()));
        let efs = reset_external(ExternalStorage::new(self.efs.clone()));
        let vfs = reset_volatile(VolatileStorage::default());

        Self::Store::init_raw(ifs, efs, vfs);
    }
}

unsafe fn reset_internal(
    mut ifs: InternalStorage,
) -> &'static Filesystem<'static, InternalStorage> {
    if ifs.format() {
        Filesystem::format(&mut ifs).expect("failed to format storage");
    }
    let ifs_storage = INTERNAL_STORAGE.insert(ifs);
    let ifs_alloc = INTERNAL_FS_ALLOC.insert(Filesystem::allocate());
    let fs = Filesystem::mount(ifs_alloc, ifs_storage).expect("failed to mount IFS");
    INTERNAL_FS.insert(fs)
}

unsafe fn reset_external(
    mut efs: ExternalStorage,
) -> &'static Filesystem<'static, ExternalStorage> {
    if efs.format() {
        Filesystem::format(&mut efs).expect("failed to format storage");
    }
    let efs_storage = EXTERNAL_STORAGE.insert(efs);
    let efs_alloc = EXTERNAL_FS_ALLOC.insert(Filesystem::allocate());
    let fs = Filesystem::mount(efs_alloc, efs_storage).expect("failed to mount EFS");
    EXTERNAL_FS.insert(fs)
}

unsafe fn reset_volatile(
    mut vfs: VolatileStorage,
) -> &'static Filesystem<'static, VolatileStorage> {
    Filesystem::format(&mut vfs).expect("failed to format VFS");
    let vfs_storage = VOLATILE_STORAGE.insert(vfs);
    let vfs_alloc = VOLATILE_FS_ALLOC.insert(Filesystem::allocate());
    let fs = Filesystem::mount(vfs_alloc, vfs_storage).expect("failed to mount VFS");
    VOLATILE_FS.insert(fs)
}
