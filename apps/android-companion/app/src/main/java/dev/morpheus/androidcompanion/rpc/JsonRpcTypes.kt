package dev.morpheus.androidcompanion.rpc

import kotlinx.serialization.json.JsonElement

data class RpcNotification(
    val method: String,
    val params: JsonElement?,
)

data class RpcError(
    val code: Long,
    override val message: String,
    val data: JsonElement? = null,
) : Exception(message)

class RpcConnectionException(message: String, cause: Throwable? = null) : Exception(message, cause)
