use libdeflater::{DecompressionError, Decompressor};

pub struct McaLoader {
    data: Vec<u8>,
    decompressor: Decompressor,
    decompressed_chunk: Vec<u8>,
}

impl Default for McaLoader {
    fn default() -> Self {
        Self {
            data: Vec::with_capacity(12000000),
            decompressor: Decompressor::new(),
            decompressed_chunk: vec![0; 80000],
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum McaError {
    #[error("invalid header")]
    InvalidHeader,

    #[error("unexpected EOF")]
    UnexpectedEof,

    #[error("chunk not found")]
    ChunkNotFound,

    #[error("unexpected compression type")]
    UnexpectedCompression,

    #[error("bad data")]
    BadData,

    #[error("invalid header: {:?}", .0)]
    DecompressionError(#[from] libdeflater::DecompressionError),

    #[error("Io error: {:?}", .0)]
    IoError(#[from] std::io::Error),
}

impl McaLoader {
    pub fn load_mca<R: std::io::Read>(&mut self, mut read: R) -> Result<(), McaError> {
        self.data.clear();
        let n = read.read_to_end(&mut self.data)?;
        if n < 8192 {
            return Err(McaError::InvalidHeader);
        }
        Ok(())
    }
    pub fn raw_bytes(&self) -> &[u8] {
        &self.data
    }
    fn get_chunk_pos(&self, idx: usize) -> Result<(u32, u8), McaError> {
        if self.data.is_empty() {
            panic!("no data loaded")
        }
        if idx >= 1024 {
            panic!("index out of bounds")
        }

        if self.data.len() < idx * 4 + 4 {
            return Err(McaError::UnexpectedEof);
        }

        let v = u32::from_be_bytes([
            self.data[idx * 4],
            self.data[idx * 4 + 1],
            self.data[idx * 4 + 2],
            self.data[idx * 4 + 3],
        ]);
        let off = v >> 8;
        let len = (v & 0xff) as u8;
        Ok((off, len))
    }
    pub fn get_chunk_data(&mut self, idx: usize) -> Result<&[u8], McaError> {
        let (off, len) = self.get_chunk_pos(idx)?;

        if off < 2 || len == 0 {
            return Err(McaError::ChunkNotFound);
        }

        let o = off as usize;

        if self.data.len() < o * 4096 + 5 {
            return Err(McaError::UnexpectedEof);
        }

        let bytes_len = i32::from_be_bytes([
            self.data[o * 4096],
            self.data[o * 4096 + 1],
            self.data[o * 4096 + 2],
            self.data[o * 4096 + 3],
        ]);

        if bytes_len <= 1 {
            return Err(McaError::BadData);
        }

        if self.data[o * 4096 + 4] != 2 {
            // 2 is zlib compression
            return Err(McaError::UnexpectedCompression);
        }

        let chunk_byte_off = o * 4096 + 5;
        let chunk_byte_len = (bytes_len - 1) as usize;

        if self.data.len() < chunk_byte_off + chunk_byte_len {
            return Err(McaError::UnexpectedEof);
        }

        let n = loop {
            match self.decompressor.zlib_decompress(
                &self.data[chunk_byte_off..chunk_byte_off + chunk_byte_len],
                &mut self.decompressed_chunk,
            ) {
                Ok(n) => break n,
                Err(DecompressionError::InsufficientSpace) => {
                    self.decompressed_chunk
                        .resize(self.decompressed_chunk.len() * 2 + 1, 0);
                }
                Err(e) => return Err(e.into()),
            }
        };

        if n <= 1 {
            return Err(McaError::BadData);
            // could have a stronger bound here, but I just need this to be able to safely access
            // index 0 of the decompressed data and to subtract 1 from n
        }

        if self.decompressed_chunk[0] != 10 || self.decompressed_chunk[n - 1] != 0 {
            // extra defensive check:
            // first byte should always be 10 (compound tag byte of the root compound tag)
            // last byte should always be 0 (end tag byte closing the root compound tag)
            return Err(McaError::BadData);
        }

        Ok(&self.decompressed_chunk[..n])
    }
}
