use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::time::Duration;

use crate::error::{Error, Result};

#[derive(Debug, PartialEq, Eq)]
pub struct Config {
    pub remission_count: usize,
    pub mtu: usize,
    pub recv_timeout: Duration,

    #[cfg(feature = "encryption")]
    pub key: [u8; 32],
}

impl Default for Config {
    fn default() -> Self {
        Self {
            remission_count: 3,
            mtu: 1024,
            recv_timeout: Duration::from_secs(3),
            #[cfg(feature = "encryption")]
            key: [0u8; 32],
        }
    }
}

enum Line<'s> {
    Key(&'s str),
    KeyValue(&'s str, &'s str),
    Section(&'s str),
    Comment,
}

impl Config {
    pub fn from_file(file: impl AsRef<Path>) -> Result<Self> {
        let stream = std::fs::File::open(file)?;
        Self::parse_stream(stream)
    }

    fn parse_stream<S: Read>(stream: S) -> Result<Self> {
        let mut reader = BufReader::new(stream);
        let mut raw_line = String::new();
        let mut linenum = 0usize;

        let mut config = Self::default();

        loop {
            // We use `read_line` instead of `lines` to avoid re-allocations
            raw_line.clear();
            let size = reader.read_line(&mut raw_line)?;
            linenum += 1;
            if size == 0 {
                break;
            }

            let line = raw_line.trim_end();
            if line.is_empty() {
                continue;
            }

            let result = Self::parse_line(&line).ok_or_else(|| Error::InvalidConfig {
                linenum,
                line: String::from(line),
            })?;
            match result {
                Line::Key(key) => {
                    log::warn!("Unknown key {:?}", key);
                }
                Line::KeyValue(key, value) => {
                    if key.eq_ignore_ascii_case("remission_count") {
                        config.remission_count = value.parse()?;
                    } else if key.eq_ignore_ascii_case("mtu") {
                        config.mtu = value.parse()?;
                    } else if key.eq_ignore_ascii_case("recv_timeout") {
                        config.recv_timeout = Duration::from_secs(value.parse()?);
                    } else if key.eq_ignore_ascii_case("key") {
                        todo!("parse key");
                    } else {
                        log::warn!("Unknown key {:?}", key);
                    }
                }
                _ => {}
            }
        }

        Ok(config)
    }

    fn parse_line<'s>(line: &'s str) -> Option<Line<'s>> {
        const COMMENT_CHARS: [char; 2] = ['#', ';'];

        fn is_valid_key(key: &str) -> bool {
            let invalid =
                key.contains(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-');
            !invalid
        }

        if line.starts_with(&COMMENT_CHARS[..]) {
            Some(Line::Comment)
        } else if let Some(index) = line.find('=') {
            let key = (&line[..index]).trim_end();

            if !is_valid_key(key) {
                None
            } else {
                let value = (&line[(index + 1)..]).trim();

                if value.is_empty() {
                    None
                } else {
                    Some(Line::KeyValue(key, value))
                }
            }
        } else if line.starts_with('[') && line.ends_with(']') {
            let section = &line[1..][..line.len() - 2];
            if is_valid_key(section) {
                Some(Line::Section(section))
            } else {
                None
            }
        } else {
            let key = line.trim();
            if !is_valid_key(key) {
                None
            } else {
                Some(Line::Key(key))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_config() {
        let config_content = r#"
mtu = 1400
; A comment
remission_count = 2
# An other one
"#;

        let stream = std::io::Cursor::new(config_content);
        let maybe_config = Config::parse_stream(stream);

        assert!(maybe_config.is_ok());
        assert_eq!(
            maybe_config.unwrap(),
            Config {
                remission_count: 2,
                mtu: 1400
            }
        );
    }
}
