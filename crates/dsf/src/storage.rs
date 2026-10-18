use std::fs::File;
use std::io::{self, Cursor, Read, Seek, Write};

mod sealed {
    pub trait Sealed {}
    impl Sealed for std::fs::File {}
    impl Sealed for std::io::Cursor<Vec<u8>> {}
}

/// A seekable, resizable byte store: a [`File`] or an in-memory buffer.
///
/// Sealed so new methods can be added without a breaking change.
pub trait Storage: Read + Write + Seek + sealed::Sealed {
    /// Truncates or extends the store to `len` bytes.
    fn set_len(&mut self, len: u64) -> io::Result<()>;
    /// Flushes buffered data to durable storage where applicable.
    fn sync(&mut self) -> io::Result<()>;
}

impl Storage for File {
    fn set_len(&mut self, len: u64) -> io::Result<()> {
        File::set_len(self, len)
    }
    fn sync(&mut self) -> io::Result<()> {
        self.sync_all()
    }
}

impl Storage for Cursor<Vec<u8>> {
    fn set_len(&mut self, len: u64) -> io::Result<()> {
        let len = usize::try_from(len)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "length exceeds usize"))?;
        self.get_mut().resize(len, 0);
        Ok(())
    }
    fn sync(&mut self) -> io::Result<()> {
        Ok(())
    }
}
