// SPDX-License-Identifier: GPL-3.0-or-later

//! The module contains the implementation of the TCP collector and reporter.

use super::Execution;
use super::reporter::{Reporter, ReporterError};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The serializer for executions to transmit over the network.
///
/// The executions are serialized using LV (Length-Value) format.
/// The length is a 4-byte big-endian integer, and the value is the JSON
/// representation of the execution.
struct ExecutionWireSerializer;

impl ExecutionWireSerializer {
    /// Read an execution from a reader using LV format.
    fn read(reader: &mut impl Read) -> Result<Execution, ReporterError> {
        let mut length_bytes = [0; 4];
        reader.read_exact(&mut length_bytes)?;
        let length = u32::from_be_bytes(length_bytes) as usize;

        let mut buffer = vec![0; length];
        reader.read_exact(&mut buffer)?;
        let execution = serde_json::from_slice(buffer.as_ref())?;

        Ok(execution)
    }

    /// Write an execution to a writer using LV format.
    fn write(writer: &mut impl Write, execution: Execution) -> Result<u32, ReporterError> {
        let serialized = serde_json::to_string(&execution)?;
        let bytes = serialized.into_bytes();
        let length = bytes.len() as u32;

        writer.write_all(&length.to_be_bytes())?;
        writer.write_all(&bytes)?;

        Ok(length)
    }
}

/// Represents a TCP execution collector.
pub struct CollectorOnTcp {
    shutdown: Arc<AtomicBool>,
    listener: TcpListener,
}

impl CollectorOnTcp {
    /// Creates a new TCP execution collector.
    ///
    /// The collector listens to a random port on the loopback interface.
    /// The address of the collector can be obtained by the `address` method.
    pub fn new() -> Result<(Self, SocketAddr), std::io::Error> {
        let shutdown = Arc::new(AtomicBool::new(false));
        // Try IPv4 loopback first, fall back to IPv6 loopback if IPv4 is unavailable.
        let listener = TcpListener::bind("127.0.0.1:0").or_else(|_| TcpListener::bind("[::1]:0"))?;
        let address = listener.local_addr()?;

        Ok((Self { shutdown, listener }, address))
    }

    /// Single-threaded implementation of the collector.
    ///
    /// The collector listens to the TCP port and accepts incoming connections.
    /// When a connection is accepted, the collector reads the executions from
    /// the connection and emits them.
    pub fn executions(&self) -> impl Iterator<Item = Result<Execution, ReporterError>> + '_ {
        let listener = &self.listener;
        let shutdown = &self.shutdown;

        std::iter::from_fn(move || {
            if shutdown.load(Ordering::Relaxed) {
                return None;
            }

            match listener.accept() {
                Ok((mut connection, _)) => {
                    if shutdown.load(Ordering::Relaxed) {
                        let _ = connection.shutdown(std::net::Shutdown::Both);
                        None
                    } else {
                        let result = ExecutionWireSerializer::read(&mut connection);
                        let _ = connection.shutdown(std::net::Shutdown::Both);
                        Some(result)
                    }
                }
                Err(err) => Some(Err(ReporterError::Network(err))),
            }
        })
    }

    /// Stops the collector by flipping the shutdown flag and connecting to the collector.
    ///
    /// The collector is stopped when the `produce` method sees the shutdown flag.
    /// To signal the collector to stop, we connect to the collector to unblock the
    /// `accept` call to check the shutdown flag.
    pub fn shutdown(&self) -> Result<(), ReporterError> {
        self.shutdown.store(true, Ordering::Relaxed);

        let address = self.listener.local_addr()?;
        let _ = TcpStream::connect(address).map_err(ReporterError::Network)?;
        Ok(())
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
    use std::io::Cursor;

    // Test that the serialization and deserialization works. We write the
    // executions to a buffer and read them back to check if the deserialized
    // values match the originals.
    #[test]
    fn read_write_works() {
        let mut writer = Cursor::new(vec![0; 1024]);
        for execution in fixtures::EXECUTIONS.iter() {
            let result = ExecutionWireSerializer::write(&mut writer, execution.clone());
            assert!(result.is_ok());
        }

        let mut reader = Cursor::new(writer.get_ref());
        for execution in fixtures::EXECUTIONS.iter() {
            let result = ExecutionWireSerializer::read(&mut reader);
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), execution.clone());
        }
    }

    // Test that the TCP reporter and the TCP collector work together.
    // We create a TCP collector and a TCP reporter, then we send executions
    // to the reporter and check if the collector receives them.
    //
    // A channel is used so the collector thread can signal that all
    // executions have been received, and a timeout prevents the test from
    // hanging indefinitely if delivery is broken.
    #[test]
    fn tcp_reporter_and_collectors_work() {
        let (collector, address) = CollectorOnTcp::new().unwrap();
        let collector_arc = Arc::new(collector);

        // Channel for the collector to signal "I've received everything"
        let (done_tx, done_rx) = std::sync::mpsc::sync_channel::<()>(0);

        // Start the collector in a separate thread using the executions iterator
        let collector_thread = {
            let tcp_collector = Arc::clone(&collector_arc);
            std::thread::spawn(move || {
                let mut received = Vec::new();
                for result in tcp_collector.executions() {
                    match result {
                        Ok(execution) => {
                            received.push(execution);
                            if received.len() == fixtures::EXECUTIONS.len() {
                                let _ = done_tx.send(());
                                break;
                            }
                        }
                        Err(err) => {
                            log::error!("Failed to receive execution: {err}");
                            break;
                        }
                    }
                }
                received
            })
        };

        // Send executions to the reporter.
        for execution in fixtures::EXECUTIONS.iter() {
            let reporter = ReporterOnTcp::new(address);
            let result = reporter.report(execution.clone());
            assert!(result.is_ok());
        }

        // Wait with a timeout — if delivery is broken, fail instead of hang.
        done_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("timed out waiting for collector to receive all executions");

        // Now safe to shutdown and join.
        collector_arc.shutdown().unwrap();
        let received = collector_thread.join().unwrap();

        // Assert that we received all the executions.
        assert_eq!(fixtures::EXECUTIONS.len(), received.len());
        for execution in received {
            assert!(fixtures::EXECUTIONS.contains(&execution));
        }
    }

    // Test that calling shutdown on the collector stops the executions
    // iterator. No data is sent — this purely tests the shutdown mechanism.
    #[test]
    fn tcp_collector_shutdown_stops_iterator() {
        let (collector, _address) = CollectorOnTcp::new().unwrap();
        let collector_arc = Arc::new(collector);

        let collector_thread = {
            let tcp_collector = Arc::clone(&collector_arc);
            std::thread::spawn(move || tcp_collector.executions().count())
        };

        collector_arc.shutdown().unwrap();

        let count = collector_thread.join().unwrap();
        assert_eq!(count, 0);
    }

    mod fixtures {
        use super::*;
        use std::collections::HashMap;

        pub(super) static EXECUTIONS: std::sync::LazyLock<Vec<Execution>> = std::sync::LazyLock::new(|| {
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
        });
    }
}
