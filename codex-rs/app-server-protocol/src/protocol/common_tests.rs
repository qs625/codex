use super::*;
use anyhow::Result;
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn client_response_payload_returns_jsonrpc_parts_and_client_response() -> Result<()> {
    let (request_id, result, payload) =
        ClientResponsePayload::ThreadArchive(crate::protocol::ThreadArchiveResponse {})
            .into_jsonrpc_parts_and_payload(RequestId::Integer(7))?;

    assert_eq!(request_id, RequestId::Integer(7));
    assert_eq!(result, json!({}));

    let Some(ClientResponse::ThreadArchive {
        request_id,
        response: _,
    }) = payload.and_then(|payload| payload.into_client_response(RequestId::Integer(7)))
    else {
        panic!("expected thread/archive client response");
    };
    assert_eq!(request_id, RequestId::Integer(7));
    Ok(())
}
