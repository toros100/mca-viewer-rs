use nbt::DeserializePayload;

/// convenience type to read, decompress and parse gzip compressed .dat files
/// like level.dat, player.dat etc.
pub struct DatReader {
    decompressor: libdeflater::Decompressor,
    read_scratch: Vec<u8>,
    decompress_scratch: Vec<u8>,
}

impl Default for DatReader {
    fn default() -> Self {
        Self {
            decompressor: Default::default(),

            // initial capacities based level.dat files
            read_scratch: Vec::with_capacity(4096),
            decompress_scratch: vec![0; 16384],
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DatError {
    #[error("Io error: {:?}", .0)]
    IoError(#[from] std::io::Error),

    #[error("decompression error: {:?}", .0)]
    DecompressionError(#[from] libdeflater::DecompressionError),

    #[error("deserialization error: {:?}", .0)]
    DeserializationError(#[from] nbt::DeserializationError),
}

impl DatReader {
    pub fn read<'d, L>(&'d mut self, mut r: impl std::io::Read) -> Result<L, DatError>
    where
        L: DeserializePayload<'d> + Default,
    {
        self.read_scratch.clear();
        r.read_to_end(&mut self.read_scratch)?;

        let n = loop {
            match self
                .decompressor
                .gzip_decompress(&self.read_scratch, &mut self.decompress_scratch)
            {
                Ok(n) => break n,
                Err(libdeflater::DecompressionError::InsufficientSpace) => {
                    self.decompress_scratch
                        .resize((self.decompress_scratch.len() + 1).next_power_of_two(), 0);
                }
                Err(e) => return Err(e.into()),
            }
        };

        // outer tags name always empty/meaningless
        // could return, but would just discard it at the callsite
        let (_, l, _) = nbt::from_bytes::<L>(&self.decompress_scratch[..n])?;
        Ok(l)
    }
}
