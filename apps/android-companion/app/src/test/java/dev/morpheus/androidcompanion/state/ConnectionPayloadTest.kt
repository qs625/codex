package dev.morpheus.androidcompanion.state

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ConnectionPayloadTest {
    @Test
    fun parsesTypedJsonPayload() {
        val result = parseConnectionPayload(
            """
            {
              "type": "morpheus.androidConnection",
              "version": 1,
              "endpoint": "wss://tunnel.example/root-worker",
              "token": "capability-token"
            }
            """.trimIndent(),
        )

        val payload = (result as ConnectionPayloadParseResult.Success).payload
        assertEquals("wss://tunnel.example/root-worker", payload.endpoint)
        assertEquals("capability-token", payload.token)
    }

    @Test
    fun parsesUriAndNakedWebSocketPayloads() {
        val uriPayload = (
            parseConnectionPayload(
                "morpheus://connect?endpoint=ws%3A%2F%2F192.168.1.2%3A8910&token=abc",
            ) as ConnectionPayloadParseResult.Success
            ).payload
        assertEquals("ws://192.168.1.2:8910", uriPayload.endpoint)
        assertEquals("abc", uriPayload.token)

        val nakedPayload = (
            parseConnectionPayload("ws://192.168.1.2:8910") as
                ConnectionPayloadParseResult.Success
            ).payload
        assertEquals("ws://192.168.1.2:8910", nakedPayload.endpoint)
        assertNull(nakedPayload.token)
    }

    @Test
    fun trimsEmptyTokensFromJsonPayload() {
        val result = parseConnectionPayload(
            """{"type":"morpheus.androidConnection","version":1,"endpoint":"ws://host:8910","token":"   "}""",
        )

        assertNull((result as ConnectionPayloadParseResult.Success).payload.token)
    }

    @Test
    fun rejectsInvalidPayloads() {
        assertFailure("https://example.com", "Morpheus connection payload")
        assertFailure(
            """{"type":"other","version":1,"endpoint":"ws://host:8910"}""",
            "not a Morpheus Android payload",
        )
        assertFailure(
            """{"type":"morpheus.androidConnection","version":2,"endpoint":"ws://host:8910"}""",
            "version is not supported",
        )
        assertFailure(
            """{"type":"morpheus.androidConnection","version":1}""",
            "missing an endpoint",
        )
        assertFailure(
            """{"type":"morpheus.androidConnection","version":1,"endpoint":"https://example.com"}""",
            "ws:// or wss://",
        )
        assertFailure(
            """{"type":"morpheus.androidConnection","version":1,"endpoint":"ws://host:8910","token":7}""",
            "token must be a string",
        )
    }

    @Test
    fun validatesManualEndpointDrafts() {
        assertNull(validateConnectionEndpoint("ws://192.168.1.2:8910"))
        assertNull(validateConnectionEndpoint("wss://tunnel.example/ws"))
        assertEquals(
            "Endpoint must start with ws:// or wss://.",
            validateConnectionEndpoint("https://example.com"),
        )
        assertEquals(
            "Endpoint cannot contain whitespace.",
            validateConnectionEndpoint("ws://bad host:8910"),
        )
    }

    private fun assertFailure(raw: String, messagePart: String) {
        val result = parseConnectionPayload(raw)
        assertTrue(result is ConnectionPayloadParseResult.Failure)
        assertTrue((result as ConnectionPayloadParseResult.Failure).message.contains(messagePart))
    }
}
