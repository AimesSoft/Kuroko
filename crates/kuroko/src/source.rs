use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::core::MediaSourceHint;

#[derive(Debug, Error)]
pub enum SourceError {
    #[error("io error: {0}")]
    Io(String),
    #[error("http error: {0}")]
    Http(String),
    #[error("unsupported source URI: {0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, SourceError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub length: Option<u64>,
}

impl ByteRange {
    pub fn suffix_from(start: u64) -> Self {
        Self {
            start,
            length: None,
        }
    }
}

pub trait MediaSource: Send {
    fn uri(&self) -> &str;
    fn len(&mut self) -> Result<Option<u64>>;
    fn read_range(&mut self, range: ByteRange) -> Result<Vec<u8>>;
}

#[derive(Debug)]
pub struct LocalFileSource {
    uri: String,
    path: PathBuf,
}

impl LocalFileSource {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let uri = format!("file://{}", path.display());
        Ok(Self { uri, path })
    }
}

impl MediaSource for LocalFileSource {
    fn uri(&self) -> &str {
        &self.uri
    }

    fn len(&mut self) -> Result<Option<u64>> {
        let metadata =
            std::fs::metadata(&self.path).map_err(|error| SourceError::Io(error.to_string()))?;
        Ok(Some(metadata.len()))
    }

    fn read_range(&mut self, range: ByteRange) -> Result<Vec<u8>> {
        let mut file =
            File::open(&self.path).map_err(|error| SourceError::Io(error.to_string()))?;
        file.seek(SeekFrom::Start(range.start))
            .map_err(|error| SourceError::Io(error.to_string()))?;
        let mut reader: Box<dyn Read> = match range.length {
            Some(length) => Box::new(file.take(length)),
            None => Box::new(file),
        };
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .map_err(|error| SourceError::Io(error.to_string()))?;
        Ok(bytes)
    }
}

#[derive(Debug)]
pub struct HttpRangeSource {
    uri: String,
}

impl HttpRangeSource {
    pub fn new(uri: impl Into<String>) -> Self {
        Self { uri: uri.into() }
    }
}

impl MediaSource for HttpRangeSource {
    fn uri(&self) -> &str {
        &self.uri
    }

    fn len(&mut self) -> Result<Option<u64>> {
        let response = ureq::head(&self.uri)
            .call()
            .map_err(|error| SourceError::Http(error.to_string()))?;
        Ok(response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok()))
    }

    fn read_range(&mut self, range: ByteRange) -> Result<Vec<u8>> {
        let header = match range.length {
            Some(length) if length > 0 => {
                format!("bytes={}-{}", range.start, range.start + length - 1)
            }
            _ => format!("bytes={}-", range.start),
        };
        let mut response = ureq::get(&self.uri)
            .header("Range", &header)
            .call()
            .map_err(|error| SourceError::Http(error.to_string()))?;
        let mut bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .read_to_end(&mut bytes)
            .map_err(|error| SourceError::Http(error.to_string()))?;
        Ok(bytes)
    }
}

pub fn source_from_uri(uri: &str) -> Result<Box<dyn MediaSource>> {
    source_from_uri_with_hint(uri, MediaSourceHint::Auto)
}

pub fn source_from_uri_with_hint(
    uri: &str,
    source_hint: MediaSourceHint,
) -> Result<Box<dyn MediaSource>> {
    match source_hint {
        MediaSourceHint::Auto => source_from_auto_uri(uri),
        MediaSourceHint::LocalFile => {
            Ok(Box::new(LocalFileSource::open(local_path_from_uri(uri))?))
        }
        MediaSourceHint::Http => {
            if uri.starts_with("http://") || uri.starts_with("https://") {
                Ok(Box::new(HttpRangeSource::new(uri)))
            } else {
                Err(SourceError::Unsupported(uri.to_string()))
            }
        }
    }
}

fn source_from_auto_uri(uri: &str) -> Result<Box<dyn MediaSource>> {
    if let Some(path) = uri.strip_prefix("file://") {
        return Ok(Box::new(LocalFileSource::open(path)?));
    }
    if uri.starts_with("http://") || uri.starts_with("https://") {
        return Ok(Box::new(HttpRangeSource::new(uri)));
    }
    let path = Path::new(uri);
    if path.exists() {
        return Ok(Box::new(LocalFileSource::open(path)?));
    }
    Err(SourceError::Unsupported(uri.to_string()))
}

fn local_path_from_uri(uri: &str) -> &str {
    uri.strip_prefix("file://").unwrap_or(uri)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn local_file_source_reads_ranges() {
        let path = std::env::temp_dir().join(format!("kuroko-source-{}.bin", std::process::id()));
        {
            let mut file = File::create(&path).unwrap();
            file.write_all(b"abcdef").unwrap();
        }

        let mut source = LocalFileSource::open(&path).unwrap();
        assert_eq!(source.len().unwrap(), Some(6));
        assert_eq!(
            source
                .read_range(ByteRange {
                    start: 2,
                    length: Some(3)
                })
                .unwrap(),
            b"cde"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn source_from_uri_rejects_unknown_scheme() {
        match source_from_uri("smb://example/video.mkv") {
            Ok(_) => panic!("unexpectedly accepted unsupported source"),
            Err(error) => assert!(matches!(error, SourceError::Unsupported(_))),
        }
    }

    #[test]
    fn source_hint_controls_selection() {
        let source =
            source_from_uri_with_hint("https://example.invalid/video.mp4", MediaSourceHint::Http)
                .unwrap();
        assert_eq!(source.uri(), "https://example.invalid/video.mp4");

        assert!(matches!(
            source_from_uri_with_hint("file:///tmp/video.mp4", MediaSourceHint::Http),
            Err(SourceError::Unsupported(_))
        ));
    }
}
