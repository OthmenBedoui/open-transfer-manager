//! Where a transfer goes. The relay only knows this interface.
use crate::s3::Client;
use std::sync::Arc;

pub use crate::s3::S3Error as SendError;

pub trait Destination: Send + Sync + 'static {
    /// Names the target; a saved journal only applies to the same one.
    fn describe(&self) -> String;
    fn part_size(&self, size: u64) -> u64;
    /// Prepares to receive a file and returns what identifies this attempt.
    fn start(&self, size: u64) -> Result<String, SendError>;
    /// Can an attempt saved earlier still be continued?
    fn still_valid(&self, _id: &str) -> bool {
        true
    }
    /// Parts arrive in any order. Returns a receipt to keep for `finish`.
    fn send_part(&self, id: &str, number: u32, offset: u64, data: &[u8]) -> Result<String, SendError>;
    /// Makes the file appear under its final name, all at once.
    fn finish(&self, id: &str, parts: &[(u32, String)], size: u64) -> Result<(), SendError>;
    fn cancel(&self, id: &str);
    fn store_empty(&self) -> Result<(), SendError>;
    fn stored_size(&self) -> Result<u64, SendError>;
    /// How each part's arrival is checked, for reports.
    fn verification(&self) -> &'static str;
}

pub struct S3Destination {
    client: Arc<Client>,
    bucket: String,
    key: String,
}

impl S3Destination {
    pub fn new(client: Arc<Client>, bucket: &str, key: &str) -> Self {
        S3Destination { client, bucket: bucket.to_string(), key: key.to_string() }
    }
}

impl Destination for S3Destination {
    fn describe(&self) -> String {
        format!("s3://{}/{}", self.bucket, self.key)
    }

    fn part_size(&self, size: u64) -> u64 {
        crate::s3::part_size_for(size)
    }

    fn start(&self, _size: u64) -> Result<String, SendError> {
        self.client.create_multipart(&self.bucket, &self.key)
    }

    fn send_part(&self, id: &str, number: u32, _offset: u64, data: &[u8]) -> Result<String, SendError> {
        self.client.upload_part(&self.bucket, &self.key, id, number, data)
    }

    fn finish(&self, id: &str, parts: &[(u32, String)], _size: u64) -> Result<(), SendError> {
        self.client.complete(&self.bucket, &self.key, id, parts)
    }

    fn cancel(&self, id: &str) {
        let _ = self.client.abort(&self.bucket, &self.key, id);
    }

    fn store_empty(&self) -> Result<(), SendError> {
        self.client.put_object(&self.bucket, &self.key, b"")
    }

    fn stored_size(&self) -> Result<u64, SendError> {
        self.client.head_size(&self.bucket, &self.key)
    }

    fn verification(&self) -> &'static str {
        "parts-signed"
    }
}
