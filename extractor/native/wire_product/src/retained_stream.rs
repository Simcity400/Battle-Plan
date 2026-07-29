use crate::model::ProductError;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use zstd::stream::raw::{
    DParameter, Decoder as RawDecoder, InBuffer, Operation, OutBuffer,
};

const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];
const BUFFER_BYTES: usize = 1 << 20;
const ZSTD_WINDOW_LOG: u32 = 27;

pub(crate) fn open(file: &File) -> Result<Box<dyn Read>, ProductError> {
    let mut cloned = file
        .try_clone()
        .map_err(|error| ProductError::new(format!("clone retained H4CR file: {error}")))?;
    cloned
        .seek(SeekFrom::Start(0))
        .map_err(|error| ProductError::new(format!("rewind retained H4CR file: {error}")))?;
    let mut magic = [0u8; 4];
    let mut filled = 0;
    while filled < magic.len() {
        let read = cloned
            .read(&mut magic[filled..])
            .map_err(|error| ProductError::new(format!("probe retained H4CR file: {error}")))?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    cloned
        .seek(SeekFrom::Start(0))
        .map_err(|error| ProductError::new(format!("rewind retained H4CR file: {error}")))?;
    let buffered = BufReader::with_capacity(BUFFER_BYTES, cloned);
    if filled == magic.len() && magic == ZSTD_MAGIC {
        Ok(Box::new(StrictZstdReader::new(buffered).map_err(
            |error| ProductError::new(format!("open retained zstd H4CR: {error}")),
        )?))
    } else {
        Ok(Box::new(buffered))
    }
}

struct StrictZstdReader<R: BufRead> {
    reader: R,
    decoder: RawDecoder<'static>,
    finished_frame: bool,
    done: bool,
}

impl<R: BufRead> StrictZstdReader<R> {
    fn new(reader: R) -> std::io::Result<Self> {
        let mut decoder = RawDecoder::new()?;
        decoder.set_parameter(DParameter::WindowLogMax(ZSTD_WINDOW_LOG))?;
        Ok(Self {
            reader,
            decoder,
            finished_frame: false,
            done: false,
        })
    }
}

impl<R: BufRead> Read for StrictZstdReader<R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if output.is_empty() || self.done {
            return Ok(0);
        }
        let mut drain_decoder = true;
        loop {
            if drain_decoder {
                drain_decoder = false;
                let (produced, finished) = {
                    let mut source = InBuffer::around(&[]);
                    let mut target = OutBuffer::around(output);
                    let remaining = self.decoder.run(&mut source, &mut target)?;
                    (target.pos(), remaining == 0)
                };
                self.finished_frame |= finished;
                if produced != 0 {
                    return Ok(produced);
                }
            }
            let input = self.reader.fill_buf()?;
            if input.is_empty() {
                if self.finished_frame {
                    self.done = true;
                    return Ok(0);
                }
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "incomplete retained zstd frame at physical EOF",
                ));
            }
            if self.finished_frame {
                self.decoder.reinit()?;
                self.finished_frame = false;
            }
            let (consumed, produced, finished) = {
                let mut source = InBuffer::around(input);
                let mut target = OutBuffer::around(output);
                let remaining = self.decoder.run(&mut source, &mut target)?;
                (source.pos(), target.pos(), remaining == 0)
            };
            self.reader.consume(consumed);
            self.finished_frame |= finished;
            if produced != 0 {
                return Ok(produced);
            }
            if consumed == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "retained zstd decoder made no progress",
                ));
            }
        }
    }
}
