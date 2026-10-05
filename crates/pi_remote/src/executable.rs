//! Header validation shared by the helper build script and runtime. No build dependencies needed.
//! This checks OS/CPU compatibility, not the complete executable or its shared-library dependencies.
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

/// Reject a wrong-platform embedded/override runner before it can be installed or spawned.
pub fn validate(path: &Path, os: &str, arch: &str) -> io::Result<()> {
    let mut file = File::open(path)?;
    let mut header = [0; 64];
    let valid = if file.read_exact(&mut header).is_err() {
        false
    } else {
        match (os, arch) {
            ("linux", "x86_64" | "aarch64") => {
                header[..6] == *b"\x7fELF\x02\x01"
                    && u16::from_le_bytes([header[18], header[19]])
                        == if arch == "x86_64" { 62 } else { 183 }
            }
            ("macos", "x86_64" | "aarch64") => {
                header[..4] == [0xcf, 0xfa, 0xed, 0xfe]
                    && u32::from_le_bytes(header[4..8].try_into().expect("four header bytes"))
                        == if arch == "x86_64" {
                            0x0100_0007
                        } else {
                            0x0100_000c
                        }
            }
            ("windows", "x86_64") if header[..2] == *b"MZ" => {
                let offset =
                    u32::from_le_bytes(header[60..64].try_into().expect("four header bytes"));
                let mut pe = [0; 6];
                offset >= 64
                    && file.seek(SeekFrom::Start(offset.into())).is_ok()
                    && file.read_exact(&mut pe).is_ok()
                    && pe == *b"PE\0\0\x64\x86"
            }
            _ => false,
        }
    };
    if valid {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Durable runner is not a native {os}/{arch} executable: {}. Build the PRODUCTION runner with 'npm exec --yes --package=bun@1.4.2 -- bun run build' in backend/durable on a matching OS/CPU, then rebuild the helper. Building pi-desktop-durable-fixture does not update the production runner.",
                path.display()
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn header(os: &str, arch: &str) -> Vec<u8> {
        let mut bytes = vec![0; 64];
        match os {
            "linux" => {
                bytes[..6].copy_from_slice(b"\x7fELF\x02\x01");
                let machine: u16 = if arch == "x86_64" { 62 } else { 183 };
                bytes[18..20].copy_from_slice(&machine.to_le_bytes());
            }
            "macos" => {
                bytes[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
                let cpu: u32 = if arch == "x86_64" {
                    0x0100_0007
                } else {
                    0x0100_000c
                };
                bytes[4..8].copy_from_slice(&cpu.to_le_bytes());
            }
            "windows" => {
                bytes[..2].copy_from_slice(b"MZ");
                bytes[60..64].copy_from_slice(&64u32.to_le_bytes());
                bytes.extend_from_slice(b"PE\0\0\x64\x86");
            }
            _ => unreachable!(),
        }
        bytes
    }
    #[test]
    fn runner_headers_match_only_the_requested_os_and_cpu() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runner");
        let targets = [
            ("linux", "x86_64"),
            ("linux", "aarch64"),
            ("macos", "x86_64"),
            ("macos", "aarch64"),
            ("windows", "x86_64"),
        ];
        for (os, arch) in targets {
            std::fs::write(&path, header(os, arch)).unwrap();
            for (expected_os, expected_arch) in targets {
                assert_eq!(
                    validate(&path, expected_os, expected_arch).is_ok(),
                    (os, arch) == (expected_os, expected_arch),
                    "{os}/{arch} for {expected_os}/{expected_arch}"
                );
            }
        }
    }
    #[test]
    fn linux_payload_cannot_be_embedded_in_a_mac_helper() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pi-desktop-durable");
        std::fs::write(&path, header("linux", "aarch64")).unwrap();
        let error = validate(&path, "macos", "aarch64").unwrap_err().to_string();
        assert!(error.contains("macos/aarch64"));
        assert!(error.contains("PRODUCTION"));
        assert!(error.contains("pi-desktop-durable-fixture"));
    }
    #[test]
    fn malformed_truncated_and_unsupported_runners_fail_closed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runner");
        for bytes in [vec![], vec![0; 63], vec![0; 64], b"#!/bin/sh\n".to_vec()] {
            std::fs::write(&path, bytes).unwrap();
            assert!(validate(&path, "linux", "aarch64").is_err());
        }
        let mut bytes = header("windows", "x86_64");
        bytes[60..64].copy_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        assert!(validate(&path, "windows", "x86_64").is_err());
        std::fs::write(&path, header("linux", "aarch64")).unwrap();
        assert!(validate(&path, "linux", "riscv64").is_err());
    }
}
