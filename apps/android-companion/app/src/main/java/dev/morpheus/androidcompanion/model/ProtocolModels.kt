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
)

data class ConversationThread(
    val id: String,
    val summary: ThreadSummary,
    val turns: List<ConversationTurn>,
)

data class ConversationTurn(
    val id: String,
    val status: String,
    val items: List<ConversationItem>,
)

data class ConversationItem(
    val id: String,
    val type: String,
    val title: String,
    val body: String,
    val toolPresentation: ToolPresentation? = null,
)

data class ToolPresentation(
    val summary: String,
    val status: String?,
    val details: String,
    val detailsIsTruncated: Boolean = false,
    val outputLabel: String? = null,
    val output: String? = null,
    val outputIsEmpty: Boolean = true,
    val outputIsTruncated: Boolean = false,
)

private const val SummaryTextLimit = 240
private const val BodyTextLimit = 12_000
private const val DetailsTextLimit = 8_000
private const val OutputTextLimit = 12_000
private const val UnknownFallbackLimit = 400

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
            body = boundedBody(formatUserMessage(this["content"]))
            toolPresentation = null
        }
        "agentMessage" -> {
            title = "Assistant"
            body = boundedBody(string("text").orEmpty())
            toolPresentation = null
        }
        "reasoning" -> {
            title = "Reasoning"
            body = boundedBody(
                formatStringArray(this["summary"])
                    .ifBlank { formatStringArray(this["content"]) }
                    .ifBlank { "Reasoning updated" },
            )
            toolPresentation = null
        }
        "plan" -> {
            title = "Plan"
            body = boundedBody(string("text").orEmpty())
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
            body = boundedBody(string("preview").orEmpty())
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
        toolPresentation = toolPresentation,
    )
}

fun appendBodyDeltaBounded(current: String, delta: String): String {
    return boundedTail(current + delta, BodyTextLimit, "text").text
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
    val output = boundedTail(string("aggregatedOutput"), OutputTextLimit)
    val summary = listOfNotNull(
        boundedInline(command, SummaryTextLimit).takeIf { it.isNotBlank() },
        exitCode?.let { "exit $it" } ?: status,
    ).joinToString(" • ").ifBlank { "Command execution" }
    val details = boundedSections(
        DetailsTextLimit,
        section("Command", command),
        section("Cwd", cwd),
        section("Status", status),
        section("Process ID", string("processId")),
        section("Source", fieldText("source")),
        section("Initial Wait", fieldText("initialWaitMs")?.let { "$it ms" }),
        section("Notify On", fieldText("notifyOn")),
        section("Duration", fieldText("durationMs")?.let { "$it ms" }),
        section("Exit Code", exitCode),
        section("Actions", this["commandActions"]?.let { boundedJson(it, DetailsTextLimit) }),
    )
    return ToolPresentation(
        summary = summary,
        status = status ?: exitCode?.let { "exit $it" },
        details = details.text.ifBlank { "Command execution" },
        detailsIsTruncated = details.truncated,
        outputLabel = "Output",
        output = output.text.takeIf { it.isNotEmpty() },
        outputIsEmpty = output.text.isEmpty(),
        outputIsTruncated = output.truncated,
    )
}

private fun JsonObject.commandExecutionNotificationPresentation(): ToolPresentation {
    val kind = string("kind") ?: "notification"
    val message = string("message")
    val output = boundedTail(string("output"), OutputTextLimit)
    val exitCode = fieldText("exitCode")
    val summary = listOfNotNull(
        "Command notification",
        kind,
        exitCode?.let { "exit $it" },
        message?.let { boundedInline(it, SummaryTextLimit) },
    ).joinToString(" • ")
    val status = if (kind == "exit" && exitCode != null) {
        if (exitCode == "0") "completed" else "failed"
    } else {
        "completed"
    }
    val details = boundedSections(
        DetailsTextLimit,
        section("Kind", kind),
        section("Command ID", string("commandItemId")),
        section("Exit Code", exitCode),
        section("Message", message),
        section("Created", fieldText("createdAtMs")),
    )
    return ToolPresentation(
        summary = summary.ifBlank { "Command notification" },
        status = status,
        details = details.text.ifBlank { "Command notification" },
        detailsIsTruncated = details.truncated,
        outputLabel = "Output",
        output = output.text.takeIf { it.isNotEmpty() },
        outputIsEmpty = output.text.isEmpty(),
        outputIsTruncated = output.truncated,
    )
}

private fun JsonObject.builtinToolCallPresentation(): ToolPresentation {
    val tool = string("tool") ?: string("toolName") ?: "Tool"
    val status = string("status")
    val summary = string("summary")?.let { boundedInline(it, SummaryTextLimit) }
        ?: listOfNotNull(tool, status).joinToString(" • ").ifBlank { "Tool call" }
    val details = boundedSections(
        DetailsTextLimit,
        section("Tool", tool),
        section("Status", status),
        section("Arguments", this["arguments"]?.let { boundedJson(it, DetailsTextLimit) }),
        section("Output", this["output"]?.let { boundedJson(it, DetailsTextLimit) }),
    )
    return ToolPresentation(
        summary = summary,
        status = status,
        details = details.text.ifBlank { "Tool call" },
        detailsIsTruncated = details.truncated,
    )
}

fun ConversationItem.appendToolOutputDelta(delta: String): ConversationItem {
    val presentation = toolPresentation
        ?: return copy(body = boundedTail(body + delta, OutputTextLimit).text)
    val currentOutput = presentation.output.orEmpty()
    val nextOutput = boundedTail(currentOutput + delta, OutputTextLimit)
    val nextPresentation = presentation.copy(
        output = nextOutput.text,
        outputIsEmpty = false,
        outputIsTruncated = presentation.outputIsTruncated || nextOutput.truncated,
    )
    return copy(
        body = if (body.isBlank()) boundedInline(delta, SummaryTextLimit) else body,
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
    return boundedJson(value, UnknownFallbackLimit)
}

private data class BoundedText(val text: String, val truncated: Boolean)

private fun boundedBody(value: String): String {
    return boundedTail(value, BodyTextLimit, "text").text
}

private fun boundedTail(value: String?, maxChars: Int, label: String = "output"): BoundedText {
    if (value == null) return BoundedText("", false)
    if (value.length <= maxChars) return BoundedText(value, false)
    val omitted = value.length - maxChars
    return BoundedText(
        "[truncated $omitted chars; showing latest $label]\n" + value.takeLast(maxChars),
        true,
    )
}

private fun boundedInline(value: String, maxChars: Int): String {
    val normalized = value.replace('\n', ' ').trim()
    return if (normalized.length <= maxChars) {
        normalized
    } else {
        normalized.take(maxChars) + "..."
    }
}

private fun boundedSections(maxChars: Int, vararg sections: String?): BoundedText {
    val builder = StringBuilder()
    var truncated = false
    for (section in sections.filterNotNull()) {
        if (section.isBlank()) continue
        if (builder.isNotEmpty()) {
            if (!appendWithinLimit(builder, "\n\n", maxChars)) {
                truncated = true
                break
            }
        }
        if (!appendWithinLimit(builder, section, maxChars)) {
            truncated = true
            break
        }
    }
    if (truncated) {
        appendTruncatedMarker(builder, maxChars)
    }
    return BoundedText(builder.toString(), truncated)
}

private fun boundedJson(value: JsonElement, maxChars: Int): String {
    val builder = StringBuilder()
    val truncated = !appendJsonWithinLimit(builder, value, maxChars)
    if (truncated) {
        appendTruncatedMarker(builder, maxChars)
    }
    return builder.toString()
}

private fun appendJsonWithinLimit(
    builder: StringBuilder,
    value: JsonElement,
    maxChars: Int,
): Boolean {
    return when (value) {
        is JsonPrimitive -> appendPrimitiveWithinLimit(builder, value, maxChars)
        is JsonArray -> {
            if (!appendWithinLimit(builder, "[", maxChars)) return false
            value.forEachIndexed { index, element ->
                if (index > 0 && !appendWithinLimit(builder, ",", maxChars)) return false
                if (!appendJsonWithinLimit(builder, element, maxChars)) return false
            }
            appendWithinLimit(builder, "]", maxChars)
        }
        is JsonObject -> {
            if (!appendWithinLimit(builder, "{", maxChars)) return false
            value.entries.forEachIndexed { index, entry ->
                if (index > 0 && !appendWithinLimit(builder, ",", maxChars)) return false
                if (!appendEscapedWithinLimit(builder, entry.key, maxChars)) return false
                if (!appendWithinLimit(builder, ":", maxChars)) return false
                if (!appendJsonWithinLimit(builder, entry.value, maxChars)) return false
            }
            appendWithinLimit(builder, "}", maxChars)
        }
    }
}

private fun appendPrimitiveWithinLimit(
    builder: StringBuilder,
    value: JsonPrimitive,
    maxChars: Int,
): Boolean {
    val content = value.contentOrNull ?: "null"
    if (content.length <= 64) {
        val literal = value.toString()
        if (!literal.startsWith("\"")) {
            return appendWithinLimit(builder, literal, maxChars)
        }
    }
    return appendEscapedWithinLimit(builder, content, maxChars)
}

private fun appendEscapedWithinLimit(
    builder: StringBuilder,
    text: String,
    maxChars: Int,
): Boolean {
    if (!appendWithinLimit(builder, "\"", maxChars)) return false
    for (char in text) {
        val escaped = when (char) {
            '\\' -> "\\\\"
            '"' -> "\\\""
            '\n' -> "\\n"
            '\r' -> "\\r"
            '\t' -> "\\t"
            else -> char.toString()
        }
        if (!appendWithinLimit(builder, escaped, maxChars)) return false
    }
    return appendWithinLimit(builder, "\"", maxChars)
}

private fun appendWithinLimit(builder: StringBuilder, text: String, maxChars: Int): Boolean {
    val remaining = maxChars - builder.length
    if (remaining <= 0) return false
    if (text.length <= remaining) {
        builder.append(text)
        return true
    }
    builder.append(text.take(remaining))
    return false
}

private fun appendTruncatedMarker(builder: StringBuilder, maxChars: Int) {
    val marker = "\n[truncated]"
    if (builder.length + marker.length <= maxChars + marker.length) {
        builder.append(marker)
    }
}
