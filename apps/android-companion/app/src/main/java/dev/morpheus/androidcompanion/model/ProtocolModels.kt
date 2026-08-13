package dev.morpheus.androidcompanion.model

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull

data class ThreadSummary(
    val id: String,
    val title: String,
    val preview: String,
    val agentPath: String?,
    val agentRole: String?,
    val lifecycleLabel: String,
    val updatedAt: Long?,
    val raw: JsonObject,
)

data class ConversationThread(
    val id: String,
    val summary: ThreadSummary,
    val turns: List<ConversationTurn>,
    val raw: JsonObject,
)

data class ConversationTurn(
    val id: String,
    val status: String,
    val items: List<ConversationItem>,
    val raw: JsonObject,
)

data class ConversationItem(
    val id: String,
    val type: String,
    val title: String,
    val body: String,
    val raw: JsonObject,
)

fun JsonObject.toThreadSummary(): ThreadSummary {
    val id = string("id") ?: "unknown"
    val preview = string("preview").orEmpty()
    val name = string("name")
    val agentPath = string("agentPath")
    val role = string("agentRole")
    val title = name
        ?: agentPath
        ?: preview.takeIf { it.isNotBlank() }
        ?: id.take(12)
    return ThreadSummary(
        id = id,
        title = title,
        preview = preview,
        agentPath = agentPath,
        agentRole = role,
        lifecycleLabel = lifecycleStatusLabel(this["lifecycleStatus"]),
        updatedAt = long("updatedAt"),
        raw = this,
    )
}

fun JsonObject.toConversationThread(): ConversationThread {
    val turns = this["turns"]
        ?.jsonArrayOrNull()
        ?.mapNotNull { it.jsonObjectOrNull()?.toConversationTurn() }
        .orEmpty()
    return ConversationThread(
        id = string("id") ?: "unknown",
        summary = toThreadSummary(),
        turns = turns,
        raw = this,
    )
}

fun JsonObject.toConversationTurn(): ConversationTurn {
    val items = this["items"]
        ?.jsonArrayOrNull()
        ?.mapNotNull { it.jsonObjectOrNull()?.toConversationItem() }
        .orEmpty()
    return ConversationTurn(
        id = string("id") ?: "unknown-turn",
        status = string("status") ?: "unknown",
        items = items,
        raw = this,
    )
}

fun JsonObject.toConversationItem(): ConversationItem {
    val type = string("type") ?: "unknown"
    val id = string("id") ?: "$type-${hashCode()}"
    val (title, body) = when (type) {
        "userMessage" -> "You" to formatUserMessage(this["content"])
        "agentMessage" -> "Assistant" to string("text").orEmpty().ifBlank { "..." }
        "reasoning" -> "Reasoning" to formatStringArray(this["summary"])
            .ifBlank { formatStringArray(this["content"]) }
            .ifBlank { "Reasoning updated" }
        "plan" -> "Plan" to string("text").orEmpty()
        "commandExecution" -> "Command" to listOfNotNull(
            string("command"),
            string("status")?.let { "status: $it" },
            string("aggregatedOutput"),
        ).joinToString("\n").ifBlank { "Command execution" }
        "commandExecutionNotification" -> "Command" to listOfNotNull(
            string("message"),
            string("output"),
        ).joinToString("\n").ifBlank { "Command notification" }
        "builtinToolCall" -> (string("toolName") ?: "Tool") to listOfNotNull(
            string("summary"),
            string("status")?.let { "status: $it" },
        ).joinToString("\n").ifBlank { "Tool call" }
        "contextCompaction" -> "Context compaction" to "Conversation history was compacted."
        "injectedContext" -> string("title") ?: "Context" to string("preview").orEmpty()
        "fileChange" -> "File changes" to formatFileChanges(this["changes"])
        else -> type to compactJson(this)
    }
    return ConversationItem(id = id, type = type, title = title, body = body, raw = this)
}

fun JsonObject.string(name: String): String? = this[name]?.jsonPrimitiveOrNull()?.contentOrNull

fun JsonObject.long(name: String): Long? = this[name]?.jsonPrimitiveOrNull()?.longOrNull

fun JsonElement?.jsonObjectOrNull(): JsonObject? = this as? JsonObject

private fun JsonElement.jsonArrayOrNull(): JsonArray? = this as? JsonArray

private fun JsonElement.jsonPrimitiveOrNull(): JsonPrimitive? = this as? JsonPrimitive

private fun lifecycleStatusLabel(value: JsonElement?): String {
    val primitive = value?.jsonPrimitiveOrNull()?.contentOrNull
    if (primitive != null) return primitive
    val obj = value?.jsonObjectOrNull() ?: return "unknown"
    return obj["status"]?.jsonPrimitiveOrNull()?.contentOrNull
        ?: obj["finalStatus"]?.jsonPrimitiveOrNull()?.contentOrNull
        ?: "active"
}

private fun formatUserMessage(content: JsonElement?): String {
    val parts = content?.jsonArrayOrNull() ?: return ""
    return parts.joinToString("\n") { part ->
        val obj = part.jsonObjectOrNull()
        when (obj?.string("type")) {
            "text" -> obj.string("text").orEmpty()
            "image" -> "[image] ${obj.string("url").orEmpty()}"
            "localImage" -> "[local image] ${obj.string("path").orEmpty()}"
            "skill" -> "[skill] ${obj.string("name").orEmpty()}"
            "mention" -> "[mention] ${obj.string("name").orEmpty()}"
            else -> compactJson(part)
        }
    }
}

private fun formatStringArray(value: JsonElement?): String {
    return value?.jsonArrayOrNull()
        ?.mapNotNull { it.jsonPrimitiveOrNull()?.contentOrNull }
        ?.joinToString("\n")
        .orEmpty()
}

private fun formatFileChanges(value: JsonElement?): String {
    return value?.jsonArrayOrNull()
        ?.mapNotNull { it.jsonObjectOrNull() }
        ?.joinToString("\n") { change ->
            val path = change.string("path") ?: "file"
            val kind = change.string("kind") ?: "changed"
            "$path $kind"
        }
        .orEmpty()
        .ifBlank { "Files changed" }
}

private fun compactJson(value: JsonElement): String {
    val raw = value.toString()
    return if (raw.length > 400) raw.take(400) + "..." else raw
}
