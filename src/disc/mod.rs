//! Disc type related logic (GameCube, Wii)

use std::{fmt::Debug, io};

use binrw::{BinRead, BinReaderExt, NullString};

use crate::{
    disc::{gcn::DiscGCN, wii::DiscWii},
    fst::{Node, NodeType},
    io::DiscIO,
    streams::{ReadStream, SharedWindowedReadStream},
    Error, Result,
};

pub(crate) mod gcn;
pub(crate) mod wii;

/// Shared GameCube & Wii disc header
#[derive(Clone, Debug, PartialEq, BinRead)]
pub struct Header {
    /// Game ID (e.g. GM8E01 for Metroid Prime)
    pub game_id: [u8; 6],
    /// Used in multi-disc games
    pub disc_num: u8,
    /// Disc version
    pub disc_version: u8,
    /// Audio streaming enabled (bool)
    pub audio_streaming: u8,
    /// Audio streaming buffer size
    pub audio_stream_buf_size: u8,
    #[br(pad_before(14))]
    /// If this is a Wii disc, this will be 0x5D1C9EA3
    pub wii_magic: u32,
    /// If this is a GameCube disc, this will be 0xC2339F3D
    pub gcn_magic: u32,
    /// Game title
    #[br(pad_size_to(64), map = NullString::into_string)]
    pub game_title: String,
    /// Disable hash verification
    pub disable_hash_verification: u8,
    /// Disable disc encryption and H3 hash table loading and verification
    pub disable_disc_enc: u8,
    /// Debug monitor offset
    #[br(pad_before(0x39e))]
    pub debug_mon_off: u32,
    /// Debug monitor load address
    pub debug_load_addr: u32,
    #[br(pad_before(0x18))]
    /// Offset to main DOL (Wii: >> 2)
    pub dol_off: u32,
    /// Offset to file system table (Wii: >> 2)
    pub fst_off: u32,
    /// File system size
    pub fst_sz: u32,
    /// File system max size
    pub fst_max_sz: u32,
    /// File system table load address
    pub fst_memory_address: u32,
    /// User position
    pub user_position: u32,
    /// User size
    #[br(pad_after(4))]
    pub user_sz: u32,
}

#[derive(Debug, PartialEq, BinRead, Copy, Clone)]
pub(crate) struct BI2Header {
    pub(crate) debug_monitor_size: i32,
    pub(crate) sim_mem_size: i32,
    pub(crate) arg_offset: u32,
    pub(crate) debug_flag: u32,
    pub(crate) trk_address: u32,
    pub(crate) trk_size: u32,
    pub(crate) country_code: u32,
    pub(crate) unk1: u32,
    pub(crate) unk2: u32,
    pub(crate) unk3: u32,
    pub(crate) dol_limit: u32,
    #[br(pad_after(0x1fd0))]
    pub(crate) unk4: u32,
}

pub(crate) const BUFFER_SIZE: usize = 0x8000;

/// Contains a disc's header & partition information.
pub trait DiscBase: Send + Sync {
    /// Retrieves the disc's header.
    fn get_header(&self) -> &Header;

    /// Opens a new partition read stream for the first data partition.
    ///
    /// `validate_hashes`: Validate Wii disc hashes while reading (slow!)
    ///
    /// # Examples
    ///
    /// Basic usage:
    /// ```no_run
    /// use nod::disc::new_disc_base;
    /// use nod::io::new_disc_io;
    ///
    /// let mut disc_io = new_disc_io("path/to/file".as_ref())?;
    /// let disc_base = new_disc_base(disc_io.as_mut())?;
    /// let mut partition = disc_base.get_data_partition(disc_io.as_mut(), false)?;
    /// # Ok::<(), nod::Error>(())
    /// ```
    fn get_data_partition<'a>(
        &self,
        disc_io: &'a mut dyn DiscIO,
        validate_hashes: bool,
    ) -> Result<Box<dyn PartReadStream + 'a>>;
}

/// Creates a new [`DiscBase`] instance.
///
/// # Examples
///
/// Basic usage:
/// ```no_run
/// use nod::io::new_disc_io;
/// use nod::disc::new_disc_base;
///
/// let mut disc_io = new_disc_io("path/to/file".as_ref())?;
/// let disc_base = new_disc_base(disc_io.as_mut())?;
/// disc_base.get_header();
/// # Ok::<(), nod::Error>(())
/// ```
pub fn new_disc_base(disc_io: &mut dyn DiscIO) -> Result<Box<dyn DiscBase>> {
    let mut stream = disc_io.begin_read_stream(0)?;
    let header: Header = stream.read_be()?;
    if header.wii_magic == 0x5D1C9EA3 {
        Result::Ok(Box::from(DiscWii::new(stream.as_mut(), header)?))
    } else if header.gcn_magic == 0xC2339F3D {
        Result::Ok(Box::from(DiscGCN::new(header)?))
    } else {
        Result::Err(Error::DiscFormat("Invalid GC/Wii magic".to_string()))
    }
}

/// An open read stream for a disc partition.
pub trait PartReadStream: ReadStream {
    /// Seeks the read stream to the specified file system node
    /// and returns a windowed stream.
    ///
    /// # Examples
    ///
    /// Basic usage:
    /// ```no_run
    /// use nod::disc::{new_disc_base, PartHeader};
    /// use nod::fst::NodeType;
    /// use nod::io::new_disc_io;
    /// use std::io::Read;
    ///
    /// let mut disc_io = new_disc_io("path/to/file".as_ref())?;
    /// let disc_base = new_disc_base(disc_io.as_mut())?;
    /// let mut partition = disc_base.get_data_partition(disc_io.as_mut(), false)?;
    /// let header = partition.read_header()?;
    /// if let Some(NodeType::File(node)) = header.find_node("/MP3/Worlds.txt") {
    ///     let mut s = String::new();
    ///     partition.begin_file_stream(node)?.read_to_string(&mut s);
    ///     println!("{}", s);
    /// }
    /// # Ok::<(), nod::Error>(())
    /// ```
    fn begin_file_stream(&mut self, node: &Node) -> io::Result<SharedWindowedReadStream>;

    /// Reads the partition header and file system table.
    fn read_header(&mut self) -> Result<Box<dyn PartHeader>>;

    /// The ideal size for buffered reads from this partition.
    /// GameCube discs have a data block size of 0x8000,
    /// whereas Wii discs have a data block size of 0x7c00.
    fn ideal_buffer_size(&self) -> usize;
}

/// Disc partition header with file system table.
pub trait PartHeader: Debug + Send + Sync {
    /// The root node for the filesystem.
    fn root_node(&self) -> &NodeType;

    /// Finds a particular file or directory by path.
    ///
    /// # Examples
    ///
    /// Basic usage:
    /// ```no_run
    /// use nod::disc::{new_disc_base, PartHeader};
    /// use nod::fst::NodeType;
    /// use nod::io::new_disc_io;
    ///
    /// let mut disc_io = new_disc_io("path/to/file".as_ref())?;
    /// let disc_base = new_disc_base(disc_io.as_mut())?;
    /// let mut partition = disc_base.get_data_partition(disc_io.as_mut(), false)?;
    /// let header = partition.read_header()?;
    /// if let Some(NodeType::File(node)) = header.find_node("/MP1/Metroid1.pak") {
    ///     println!("{}", node.name);
    /// }
    /// if let Some(NodeType::Directory(node, children)) = header.find_node("/MP1") {
    ///     println!("Number of files: {}", children.len());
    /// }
    /// # Ok::<(), nod::Error>(())
    /// ```
    fn find_node(&self, path: &str) -> Option<&NodeType>;
}
