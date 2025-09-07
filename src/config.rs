extern crate alloc;

use crate::filesystem::AppStorage;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use littlefs2::fs::{Allocation, FileType, Filesystem};
use littlefs2::object_safe::DynFilesystem;
use littlefs2::path::{Path, PathBuf};
use log::{info, warn};
use once_cell::sync::Lazy;
use serde::de::Unexpected;
use serde::{Deserialize, Deserializer};

#[derive(Debug, Deserialize)]
pub struct Config {
    pub wifi: Wifi,
    pub net: Net,
}

#[derive(Debug, Deserialize)]
pub struct Wifi {
    pub ssid: String,
    pub password: String,
    pub channel: Option<u8>,
}

#[derive(Debug, Deserialize)]
pub struct Net {
    pub https: Https,
}

#[derive(Debug, Deserialize)]
pub struct Https {
    pub ca_cert: CaCert,
}

#[derive(Debug)]
pub struct CaCert {
    pub pem: Vec<u8>,
}

impl<'de> Deserialize<'de> for CaCert {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        toml::map::Map::deserialize(deserializer).and_then(|map| {
            let value = map.get_key_value("pem");
            match value {
                None => Err(serde::de::Error::missing_field("pem")),
                Some((_k, v)) => match v.as_str() {
                    None => Err(serde::de::Error::invalid_type(
                        Unexpected::Other(v.type_str()),
                        &"string",
                    )),
                    Some(s) => {
                        let mut data = s.as_bytes().to_vec();
                        data.push(0);
                        Ok(CaCert { pem: data })
                    }
                },
            }
        })
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            wifi: Wifi {
                ssid: "Wokwi-GUEST".to_string(),
                password: "".to_string(),
                channel: Some(6u8),
            },
            net: Net {
                https: Https {
                    ca_cert: CaCert {
                        pem: crate::net::ca_certs::LETS_ENCRYPT_ISRG_ROOT_X1.to_vec(),
                    },
                },
            },
        }
    }
}

fn list(fs: &dyn DynFilesystem, path: &Path) {
    fs.read_dir_and_then(path, &mut |iter| {
        for entry in iter {
            let entry = entry.unwrap();
            match entry.file_type() {
                FileType::File => info!("F {}", entry.path()),
                FileType::Dir => match entry.file_name().as_str() {
                    "." => (),
                    ".." => (),
                    _ => {
                        info!("D {}", entry.path());
                        list(fs, entry.path());
                    }
                },
            }
        }
        Ok(())
    })
    .unwrap()
}
pub static CONFIG: Lazy<Config> = Lazy::new(|| {
    let mut storage = AppStorage::new();
    let mut alloc = Allocation::new();
    let fs = Filesystem::mount(&mut alloc, &mut storage)
        .map(Some)
        .unwrap_or(None);
    
    match fs {
        None => {
            Filesystem::format(&mut storage).unwrap();
            warn!("No filesystem available. Formatted it and using default config");
            Config::default()
        }
        Some(fs) => {
            let read_result =
                fs.read::<{ 1024 * 4 }>(Path::from_bytes_with_nul(b"/config.toml\0").unwrap());
            match read_result {
                Ok(d) => toml::from_slice(d.as_slice()).unwrap_or_else(|e| {
                    warn!("Failed to parse config: {e:?}");
                    warn!("Using default config");
                    Config::default()
                }),
                Err(e) => {
                    fs.write(
                        Path::from_bytes_with_nul(b"/config.toml\0").unwrap(),
                        include_bytes!("../config.toml"),
                    )
                    .unwrap();
                    warn!("Failed to read config: {:?}", e.code());
                    let path = PathBuf::new();
                    list(&fs, &path);
                    warn!("Using default config");
                    Config::default()
                }
            }
        }
    }
});
