// SPDX-License-Identifier: GPL-3.0-or-later

//! The module contains the implementation of the TCP collector and reporter.

use crate::Execution;
use crate::reporter::{Reporter, ReporterError};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};

/// The serializer for executions to transmit over the network.
///
/// The executions are serialized using LV (Length-Value) format.
/// The length is a 4-byte big-endian integer, and the value is the JSON
/// representation of the execution.
///
/// Both the reporter (sender, in this crate) and the collector (receiver, in
/// `intercept-supervisor`) share this wire format, so the type is `pub`.
pub struct ExecutionWireSerializer;

impl ExecutionWireSerializer {
    /// Read an execution from a reader using LV format.
    pub fn read(reader: &mut impl Read) -> Result<Execution, ReporterError> {
        let mut length_bytes = [0; 4];
        reader.read_exact(&mut length_bytes)?;
        let length = u32::from_be_bytes(length_bytes) as usize;

        let mut buffer = vec![0; length];
        reader.read_exact(&mut buffer)?;
        let execution = serde_json::from_slice(buffer.as_ref())?;

        Ok(execution)
    }

    /// Write an execution to a writer using LV format.
    pub fn write(writer: &mut impl Write, execution: Execution) -> Result<u32, ReporterError> {
        let serialized = serde_json::to_string(&execution)?;
        let bytes = serialized.into_bytes();
        let length = bytes.len() as u32;

        writer.write_all(&length.to_be_bytes())?;
        writer.write_all(&bytes)?;

        Ok(length)
    }
}

/// Represents a TCP execution reporter.
pub struct ReporterOnTcp {
    destination: SocketAddr,
}

impl ReporterOnTcp {
    /// Creates a new TCP reporter instance.
    ///
    /// It does not open the TCP connection yet. Stores the destination
    /// address and creates a unique reporter id.
    pub fn new(destination: SocketAddr) -> Self {
        Self { destination }
    }
}

impl Reporter for ReporterOnTcp {
    /// Sends an execution to the remote collector.
    ///
    /// The execution's environment is trimmed to the variables relevant for
    /// compilation database generation before serialization.
    /// The TCP connection is opened and closed for each execution.
    fn report(&self, execution: Execution) -> Result<(), ReporterError> {
        let execution = execution.trim();
        log::debug!("Execution report: {execution:?}");

        let mut socket = TcpStream::connect(self.destination).map_err(ReporterError::Network)?;
        ExecutionWireSerializer::write(&mut socket, execution)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::Cursor;

    fn executions() -> Vec<Execution> {
        vec![
            Execution::from_strings("/usr/bin/ls", vec!["ls", "-l"], "/tmp", HashMap::new()),
            Execution::from_strings(
                "/usr/bin/cc",
                vec!["cc", "-c", "./file_a.c", "-o", "./file_a.o"],
                "/home/user",
                HashMap::from([("PATH", "/usr/bin:/bin"), ("CC", "gcc")]),
            ),
            Execution::from_strings(
                "/usr/bin/ld",
                vec!["ld", "-o", "./file_a", "./file_a.o"],
                "/opt/project",
                HashMap::from([("PATH", "/usr/bin:/bin"), ("LD_PRELOAD", "/usr/lib:/lib")]),
            ),
        ]
    }

    // Test that the serialization and deserialization works. We write the
    // executions to a buffer and read them back to check if the deserialized
    // values match the originals.
    #[test]
    fn read_write_works() {
        let executions = executions();

        let mut writer = Cursor::new(vec![0; 1024]);
        for execution in executions.iter() {
            let result = ExecutionWireSerializer::write(&mut writer, execution.clone());
            assert!(result.is_ok());
        }

        let mut reader = Cursor::new(writer.get_ref());
        for execution in executions.iter() {
            let result = ExecutionWireSerializer::read(&mut reader);
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), execution.clone());
        }
    }

    // Test that the reporter writes a record the wire serializer can read back.
    // A raw listener stands in for the collector (which lives in
    // `intercept-supervisor`); the end-to-end reporter/collector test lives
    // there.
    #[test]
    fn reporter_writes_a_readable_record() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();

        let sender = std::thread::spawn(move || {
            let reporter = ReporterOnTcp::new(address);
            reporter.report(Execution::from_strings(
                "/usr/bin/cc",
                vec!["cc", "-c", "file.c"],
                "/tmp",
                HashMap::from([("PATH", "/usr/bin:/bin")]),
            ))
        });

        let (mut connection, _) = listener.accept().unwrap();
        let received = ExecutionWireSerializer::read(&mut connection).unwrap();

        sender.join().unwrap().expect("report should succeed");
        assert_eq!(received.executable, std::path::PathBuf::from("/usr/bin/cc"));
        assert_eq!(received.arguments, vec!["cc", "-c", "file.c"]);
    }
}
