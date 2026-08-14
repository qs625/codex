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
    val toolPresentation: ToolPresentation? = null,
)

data class ToolPresentation(
    val summary: String,
    val status: String?,
    val details: String,
    val outputLabel: String? = null,
    val output: String? = null,
    val outputIsEmpty: Boolean = true,
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
    val title: String
    val body: String
    val toolPresentation: ToolPresentation?
    when (type) {
        "userMessage" -> {
            title = "You"
            body = formatUserMessage(this["content"])
            toolPresentation = null
        }
        "agentMessage" -> {
            title = "Assistant"
            body = string("text").orEmpty()
            toolPresentation = null
        }
        "reasoning" -> {
            title = "Reasoning"
            body = formatStringArray(this["summary"])
            .ifBlank { formatStringArray(this["content"]) }
            .ifBlank { "Reasoning updated" }
            toolPresentation = null
        }
        "plan" -> {
            title = "Plan"
            body = string("text").orEmpty()
            toolPresentation = null
        }
        "commandExecution" -> {
            title = "Command"
            toolPresentation = commandExecutionPresentation()
            body = toolPresentation.summary
        }
        "commandExecutionNotification" -> {
            title = "Command"
            toolPresentation = commandExecutionNotificationPresentation()
            body = toolPresentation.summary
        }
        "builtinToolCall" -> {
            title = string("tool") ?: string("toolName") ?: "Tool"
            toolPresentation = builtinToolCallPresentation()
            body = toolPresentation.summary
        }
        "contextCompaction" -> {
            title = "Context compaction"
            body = "Conversation history was compacted."
            toolPresentation = null
        }
        "injectedContext" -> {
            title = string("title") ?: "Context"
            body = string("preview").orEmpty()
            toolPresentation = null
        }
        "fileChange" -> {
            title = "File changes"
            body = formatFileChanges(this["changes"])
            toolPresentation = null
        }
        else -> {
            title = type
            body = compactJson(this)
            toolPresentation = null
        }
    }
    return ConversationItem(
        id = id,
        type = type,
        title = title,
        body = body,
        raw = this,
        toolPresentation = toolPresentation,
    )
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

private fun JsonObject.commandExecutionPresentation(): ToolPresentation {
    val command = string("command").orEmpty()
    val cwd = string("cwd").orEmpty()
    val status = string("status")
    val exitCode = fieldText("exitCode")
    val output = string("aggregatedOutput")
    val summary = listOfNotNull(
        command.takeIf { it.isNotBlank() },
        exitCode?.let { "exit $it" } ?: status,
    ).joinToString(" • ").ifBlank { "Command execution" }
    val details = listOfNotNull(
        section("Command", command),
        section("Cwd", cwd),
        section("Status", status),
        section("Process ID", string("processId")),
        section("Source", fieldText("source")),
        section("Initial Wait", fieldText("initialWaitMs")?.let { "$it ms" }),
        section("Notify On", fieldText("notifyOn")),
        section("Duration", fieldText("durationMs")?.let { "$it ms" }),
        section("Exit Code", exitCode),
        section("Actions", this["commandActions"]?.let { compactJson(it) }),
    ).joinToString("\n\n")
    return ToolPresentation(
        summary = summary,
        status = status ?: exitCode?.let { "exit $it" },
        details = details.ifBlank { "Command execution" },
        outputLabel = "Output",
        output = output,
        outputIsEmpty = output.isNullOrEmpty(),
    )
}

private fun JsonObject.commandExecutionNotificationPresentation(): ToolPresentation {
    val kind = string("kind") ?: "notification"
    val message = string("message")
    val output = string("output")
    val exitCode = fieldText("exitCode")
    val summary = listOfNotNull(
        "Command notification",
        kind,
        exitCode?.let { "exit $it" },
        message?.take(120),
    ).joinToString(" • ")
    val status = if (kind == "exit" && exitCode != null) {
        if (exitCode == "0") "completed" else "failed"
    } else {
        "completed"
    }
    val details = listOfNotNull(
        section("Kind", kind),
        section("Command ID", string("commandItemId")),
        section("Exit Code", exitCode),
        section("Message", message),
        section("Created", fieldText("createdAtMs")),
    ).joinToString("\n\n")
    return ToolPresentation(
        summary = summary.ifBlank { "Command notification" },
        status = status,
        details = details.ifBlank { "Command notification" },
        outputLabel = "Output",
        output = output,
        outputIsEmpty = output.isNullOrEmpty(),
    )
}

private fun JsonObject.builtinToolCallPresentation(): ToolPresentation {
    val tool = string("tool") ?: string("toolName") ?: "Tool"
    val status = string("status")
    val summary = string("summary")
        ?: listOfNotNull(tool, status).joinToString(" • ").ifBlank { "Tool call" }
    val details = listOfNotNull(
        section("Tool", tool),
        section("Status", status),
        section("Arguments", this["arguments"]?.toString()),
        section("Output", this["output"]?.toString()),
    ).joinToString("\n\n")
    return ToolPresentation(
        summary = summary,
        status = status,
        details = details.ifBlank { "Tool call" },
    )
}

fun ConversationItem.appendToolOutputDelta(delta: String): ConversationItem {
    val presentation = toolPresentation ?: return copy(body = body + delta)
    val currentOutput = presentation.output.orEmpty()
    val nextPresentation = presentation.copy(
        output = currentOutput + delta,
        outputIsEmpty = false,
    )
    return copy(
        body = if (body.isBlank()) delta else body,
        toolPresentation = nextPresentation,
    )
}

private fun JsonObject.fieldText(name: String): String? {
    return this[name]?.jsonPrimitiveOrNull()?.contentOrNull
}

private fun section(label: String, value: String?): String? {
    val text = value?.takeIf { it.isNotBlank() } ?: return null
    return "$label\n$text"
}

private fun compactJson(value: JsonElement): String {
    val raw = value.toString()
    return if (raw.length > 400) raw.take(400) + "..." else raw
}
