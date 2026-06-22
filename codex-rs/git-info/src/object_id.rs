/// Compute the Git SHA-1 blob object ID for the given content bytes.
pub fn git_blob_oid(data: &[u8]) -> String {
    let header = format!("blob {}\0", data.len());
    use sha1::Digest;
    let mut hasher = sha1::Sha1::new();
    hasher.update(header.as_bytes());
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::git_blob_oid;

    #[test]
    fn computes_git_blob_oid_with_blob_header() {
        assert_eq!(
            git_blob_oid(b"hello\n"),
            "ce013625030ba8dba906f756967f9e9ca394464a"
        );
    }

    #[test]
    fn distinguishes_raw_sha1_from_git_blob_oid() {
        assert_ne!(
            git_blob_oid(b"hello\n"),
            "f572d396fae9206628714fb2ce00f72e94f2258f"
        );
    }
}
