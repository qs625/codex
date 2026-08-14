package dev.morpheus.androidcompanion.state

import java.net.URI
import java.net.URLDecoder
import java.nio.charset.StandardCharsets
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonObject

private const val PayloadType = "morpheus.androidConnection"
private const val PayloadVersion = 1

data class ConnectionPayload(
    val endpoint: String,
    val token: String?,
)

sealed interface ConnectionPayloadParseResult {
    data class Success(val payload: ConnectionPayload) : ConnectionPayloadParseResult
    data class Failure(val message: String) : ConnectionPayloadParseResult
}

fun parseConnectionPayload(raw: String): ConnectionPayloadParseResult {
    val text = raw.trim()
    if (text.isEmpty()) {
        return ConnectionPayloadParseResult.Failure("Connection QR is empty.")
    }
    return when {
        text.startsWith("{") -> parseJsonConnectionPayload(text)
        text.startsWith("morpheus://") -> parseUriConnectionPayload(text)
        isValidWebSocketEndpoint(text) -> ConnectionPayloadParseResult.Success(
            ConnectionPayload(endpoint = text, token = null),
        )
        else -> ConnectionPayloadParseResult.Failure(
            "Connection QR must be a Morpheus connection payload.",
        )
    }
}

fun validateConnectionEndpoint(endpoint: String): String? {
    val normalizedEndpoint = endpoint.trim()
    if (normalizedEndpoint.isEmpty()) {
        return "Enter a WebSocket URL."
    }
    if (normalizedEndpoint.any { it.isWhitespace() }) {
        return "Endpoint cannot contain whitespace."
    }
    return if (isValidWebSocketEndpoint(normalizedEndpoint)) {
        null
    } else {
        "Endpoint must start with ws:// or wss://."
    }
}

private fun parseJsonConnectionPayload(text: String): ConnectionPayloadParseResult {
    val jsonObject = try {
        Json.parseToJsonElement(text).jsonObject
    } catch (_: Throwable) {
        return ConnectionPayloadParseResult.Failure("Connection QR contains invalid JSON.")
    }
    val type = jsonObject.stringValue("type")
    if (type != PayloadType) {
        return ConnectionPayloadParseResult.Failure(
            "Connection QR is not a Morpheus Android payload.",
        )
    }
    val version = jsonObject.intValue("version")
    if (version != PayloadVersion) {
        return ConnectionPayloadParseResult.Failure(
            "Connection QR payload version is not supported.",
        )
    }
    val endpoint = jsonObject.stringValue("endpoint")
        ?: return ConnectionPayloadParseResult.Failure(
            "Connection QR payload is missing an endpoint.",
        )
    val tokenElement = jsonObject["token"]
    val token = when (tokenElement) {
        null -> null
        is JsonPrimitive -> tokenElement.stringContentOrNull()
            ?: return ConnectionPayloadParseResult.Failure(
                "Connection QR token must be a string.",
            )
        else -> return ConnectionPayloadParseResult.Failure(
            "Connection QR token must be a string.",
        )
    }
    return normalizePayload(endpoint, token)
}

private fun parseUriConnectionPayload(text: String): ConnectionPayloadParseResult {
    val uri = try {
        URI(text)
    } catch (_: Throwable) {
        return ConnectionPayloadParseResult.Failure("Connection URI is invalid.")
    }
    if (uri.scheme != "morpheus" || uri.host != "connect") {
        return ConnectionPayloadParseResult.Failure(
            "Connection URI is not a Morpheus connect URI.",
        )
    }
    val params = parseQueryParams(uri.rawQuery.orEmpty())
    val endpoint = params["endpoint"]
        ?: return ConnectionPayloadParseResult.Failure(
            "Connection URI is missing an endpoint.",
        )
    return normalizePayload(endpoint, params["token"])
}

private fun normalizePayload(
    endpoint: String,
    token: String?,
): ConnectionPayloadParseResult {
    val normalizedEndpoint = endpoint.trim()
    validateConnectionEndpoint(normalizedEndpoint)?.let {
        return ConnectionPayloadParseResult.Failure(it)
    }
    val normalizedToken = token?.trim().orEmpty()
    return ConnectionPayloadParseResult.Success(
        ConnectionPayload(
            endpoint = normalizedEndpoint,
            token = normalizedToken.ifEmpty { null },
        ),
    )
}

private fun isValidWebSocketEndpoint(endpoint: String): Boolean {
    val uri = try {
        URI(endpoint)
    } catch (_: Throwable) {
        return false
    }
    return (uri.scheme == "ws" || uri.scheme == "wss") &&
        !uri.host.isNullOrBlank() &&
        endpoint.none { it.isWhitespace() }
}

private fun JsonObject.stringValue(key: String): String? {
    val value = get(key) as? JsonPrimitive ?: return null
    return value.stringContentOrNull()
}

private fun JsonObject.intValue(key: String): Int? {
    val value = get(key) as? JsonPrimitive ?: return null
    return if (value.stringContentOrNull() != null) null else value.intOrNull
}

private fun JsonPrimitive.stringContentOrNull(): String? {
    return if (toString().startsWith("\"")) contentOrNull else null
}

private fun parseQueryParams(query: String): Map<String, String> {
    if (query.isBlank()) {
        return emptyMap()
    }
    return query.split("&").mapNotNull { pair ->
        val parts = pair.split("=", limit = 2)
        val key = decode(parts.firstOrNull().orEmpty())
        if (key.isBlank()) {
            null
        } else {
            key to decode(parts.getOrElse(1) { "" })
        }
    }.toMap()
}

private fun decode(value: String): String {
    return URLDecoder.decode(value, StandardCharsets.UTF_8.name())
}
