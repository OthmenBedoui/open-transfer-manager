//! Open Transfer Manager: transfers over HTTP ranges and to object storage, and a server
//! that misbehaves on purpose so the engine can be measured under difficult conditions.
pub mod destination;
pub mod engine;
pub mod fake_s3;
pub mod fake_sftp;
pub mod lab;
pub mod ranges;
pub mod relay;
pub mod s3;
pub mod sftp;
