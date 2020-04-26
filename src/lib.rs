#![warn(missing_docs)]

//! `BufRead` and `Write`r detects compression algorithms from file extension.
//!
//! Supported formats:
//! * Gzip (`.gz`) by [`flate2`](https://crates.io/crates/flate2) crate
//! * LZ4 (`.lz4`) by [`lz4`](https://crates.io/crates/lz4) crate

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Error, ErrorKind, Read, Result, Write};
use std::path::Path;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use lz4::liblz4::ContentChecksum;
use lz4::{Decoder as Lz4Decoder, Encoder as Lz4Encoder, EncoderBuilder as Lz4EncoderBuilder};

/// The [`BufRead`](https://doc.rust-lang.org/std/io/trait.BufRead.html) type reads from compressed or uncompressed file.
///
/// This reader detects compression algorithms from file name extension.
pub struct DetectReader {
    inner: Box<dyn BufRead>,
}

impl DetectReader {
    /// Open compressed or uncompressed file.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<DetectReader> {
        DetectReader::open_with_wrapper::<P, Id>(path, Id)
    }

    /// Open compressed or uncompressed file using wrapper type.
    ///
    /// [`InnerReadWrapper`](trait.InnerReadWrapper.html) is the wrapepr type's trait handles compressed byte stream.
    /// For example, the progress-counting wrapper enables you to calculate progress of loading.
    pub fn open_with_wrapper<P: AsRef<Path>, B: ReadWrapperBuilder>(
        path: P,
        builder: B,
    ) -> Result<DetectReader> {
        let path = path.as_ref();

        let f = File::open(path)?;
        let wf = builder.new_wrapped_reader(f);

        let inner: Box<dyn BufRead> = match path.extension() {
            Some(e) if e == "gz" => {
                let d = GzDecoder::new(wf);
                let br = BufReader::new(d);
                Box::new(br)
            }
            Some(e) if e == "lz4" => {
                let d = Lz4Decoder::new(wf)?;
                let br = BufReader::new(d);
                Box::new(br)
            }
            _ => {
                let br = BufReader::new(wf);
                Box::new(br)
            }
        };

        Ok(DetectReader { inner })
    }
}

impl Read for DetectReader {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.inner.read(buf)
    }
}

impl BufRead for DetectReader {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.inner.consume(amt)
    }
}

/// The [`Write`](https://doc.rust-lang.org/std/io/trait.Write.html) type writes to compressed or uncompressed file.
///
/// This writer detects compression algorithms from file name extension.
///
/// You must [`finalize`](struct.DetectWriter.html#method.finalize) this writer.
pub struct DetectWriter {
    inner: Box<dyn Finalize>,
    not_closed: bool,
}

impl DetectWriter {
    /// Create compressed or uncompressed file.
    pub fn create<P: AsRef<Path>>(path: P, level: Level) -> Result<DetectWriter> {
        DetectWriter::create_with_wrapper::<P, Id>(path, level, Id)
    }

    /// Create compressed or uncompressed file using wrapper type.
    ///
    /// [`InnerWriteWrapper`](trait.InnerWriteWrapper.html) is the wrapepr type's trait handles compressed byte stream.
    /// For example, the size-accumulating wrapper enables you to calculate size of compressed output.
    pub fn create_with_wrapper<P: AsRef<Path>, B: WriteWrapperBuilder>(
        path: P,
        level: Level,
        builder: B,
    ) -> Result<DetectWriter> {
        let path = path.as_ref();

        let f = File::create(path)?;
        let wf = builder.new_wrapped_writer(f);
        let w = BufWriter::new(wf);

        let inner: Box<dyn Finalize> = match path.extension() {
            Some(e) if e == "gz" => {
                let e = GzEncoder::new(w, level.into_flate2_compression());
                Box::new(e)
            }
            Some(e) if e == "lz4" => {
                let mut builder = Lz4EncoderBuilder::new();
                builder
                    .level(level.into_lz4_level()?)
                    .checksum(ContentChecksum::ChecksumEnabled);

                let e = builder.build(w)?;
                Box::new(FinalizeLz4Encoder::new(e))
            }
            _ => Box::new(w),
        };

        Ok(DetectWriter {
            inner,
            not_closed: true,
        })
    }

    /// Finalize this writer.
    ///
    /// Some encodings requires finalization.
    ///
    pub fn finalize(mut self) -> Result<()> {
        if self.not_closed {
            self.inner.finalize()?;
            self.not_closed = false;
        }
        Ok(())
    }
}

impl Write for DetectWriter {
    fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        self.inner.write(bytes)
    }

    fn flush(&mut self) -> Result<()> {
        self.inner.flush()
    }
}

impl Drop for DetectWriter {
    fn drop(&mut self) {
        if self.not_closed {
            panic!("DetectWriter must be finalized. But dropped before finalization.");
        }
    }
}

/// Compression level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Uncompressed
    None,
    /// Minimum compression (fastest and large)
    Minimum,
    /// Maximum compression (smallest and slow)
    Maximum,
}

impl Level {
    fn into_flate2_compression(self) -> Compression {
        match self {
            Level::None => Compression::none(),
            Level::Minimum => Compression::fast(),
            Level::Maximum => Compression::best(),
        }
    }

    fn into_lz4_level(self) -> Result<u32> {
        match self {
            Level::None => Err(Error::new(
                ErrorKind::InvalidInput,
                "LZ4 don't support non-compression mode",
            )),
            Level::Minimum => Ok(1),
            Level::Maximum => Ok(3),
        }
    }
}

/// The [`Read`](https://doc.rust-lang.org/std/io/trait.Read.html) wrapper builder.
///
/// For more information, see [`DetectReader::open_with_wrapper()`](struct.DetectReader.html#method.open_with_wrapper).
pub trait ReadWrapperBuilder {
    /// Read wrapper of `File`
    type Wrapper: 'static + Read;
    /// Create new wrapper.
    fn new_wrapped_reader(self, f: File) -> Self::Wrapper;
}

/// The [`Write`](https://doc.rust-lang.org/std/io/trait.Write.html) wrapper builder.
///
/// For more information, see [`DetectWriter::create_with_wrapper()`](struct.DetectWriter.html#method.create_with_wrapper).
pub trait WriteWrapperBuilder {
    /// Write wrapper of `File`
    type Wrapper: 'static + Write;
    /// Create new wrapper.
    fn new_wrapped_writer(self, f: File) -> Self::Wrapper;
}

#[derive(Debug, Clone, Copy)]
struct Id;

impl ReadWrapperBuilder for Id {
    type Wrapper = File;
    fn new_wrapped_reader(self, f: File) -> Self::Wrapper {
        f
    }
}

impl WriteWrapperBuilder for Id {
    type Wrapper = File;
    fn new_wrapped_writer(self, f: File) -> Self::Wrapper {
        f
    }
}

trait Finalize: Write {
    fn finalize(&mut self) -> Result<()> {
        self.flush()
    }
}

impl Finalize for File {}
impl<W: Write> Finalize for GzEncoder<W> {}
impl<W: Write> Finalize for BufWriter<W> {}

struct FinalizeLz4Encoder<W: Write>(Option<Lz4Encoder<W>>);

impl<W: Write> FinalizeLz4Encoder<W> {
    fn new(inner: Lz4Encoder<W>) -> FinalizeLz4Encoder<W> {
        FinalizeLz4Encoder(Some(inner))
    }
}

impl<W: Write> Write for FinalizeLz4Encoder<W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.0
            .as_mut()
            .expect("writer already finalized")
            .write(buf)
    }

    fn flush(&mut self) -> Result<()> {
        self.0.as_mut().expect("writer already finalized").flush()
    }
}

impl<W: Write> Finalize for FinalizeLz4Encoder<W> {
    fn finalize(&mut self) -> Result<()> {
        self.flush()?;
        let enc = self.0.take().expect("writer already finalized");
        enc.finish().1
    }
}
