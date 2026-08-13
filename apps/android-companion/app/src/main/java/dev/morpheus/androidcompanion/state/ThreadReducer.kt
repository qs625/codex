package dev.morpheus.androidcompanion.state

import dev.morpheus.androidcompanion.model.ConversationItem
import dev.morpheus.androidcompanion.model.ConversationThread
import dev.morpheus.androidcompanion.model.ConversationTurn
import dev.morpheus.androidcompanion.model.ThreadSummary
import dev.morpheus.androidcompanion.model.jsonObjectOrNull
import dev.morpheus.androidcompanion.model.string
import dev.morpheus.androidcompanion.model.toConversationItem
import dev.morpheus.androidcompanion.model.toConversationThread
import dev.morpheus.androidcompanion.model.toConversationTurn
import dev.morpheus.androidcompanion.model.toThreadSummary
import dev.morpheus.androidcompanion.rpc.RpcNotification
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject

fun reduceThreadList(result: JsonElement): List<ThreadSummary> {
    val data = result.jsonObjectOrNull()
        ?.get("data")
        ?.let { it as? kotlinx.serialization.json.JsonArray }
        .orEmpty()
    return data.mapNotNull { it.jsonObjectOrNull()?.toThreadSummary() }
}

fun reduceThreadRead(result: JsonElement): ConversationThread? {
    return result.jsonObjectOrNull()
        ?.get("thread")
        ?.jsonObjectOrNull()
        ?.toConversationThread()
}

fun applySelectedThreadRead(
    current: CompanionUiState,
    requestedThreadId: String,
    thread: ConversationThread?,
): CompanionUiState {
    return if (current.selectedThreadId == requestedThreadId) {
        current.copy(selectedThread = thread, isReadingThread = false)
    } else {
        current
    }
}

fun applyNotification(
    threads: List<ThreadSummary>,
    selected: ConversationThread?,
    notification: RpcNotification,
): Pair<List<ThreadSummary>, ConversationThread?> {
    val params = notification.params?.jsonObjectOrNull() ?: return threads to selected
    return when (notification.method) {
        "thread/started" -> {
            val thread = params["thread"]?.jsonObjectOrNull()?.toThreadSummary()
                ?: return threads to selected
            upsertSummary(threads, thread) to selected
        }
        "thread/name/updated" -> {
            val threadId = params.string("threadId") ?: return threads to selected
            val name = params.string("threadName")
            threads.map { summary ->
                if (summary.id == threadId) summary.copy(title = name ?: summary.preview.ifBlank { summary.id.take(12) }) else summary
            } to selected
        }
        "thread/status/changed" -> {
            val threadId = params.string("threadId") ?: return threads to selected
            val status = params["lifecycleStatus"]?.toString()?.trim('"') ?: "unknown"
            threads.map { summary ->
                if (summary.id == threadId) summary.copy(lifecycleLabel = status) else summary
            } to selected
        }
        "thread/archived" -> {
            val threadId = params.string("threadId") ?: return threads to selected
            threads.filterNot { it.id == threadId } to selected?.takeUnless { it.id == threadId }
        }
        "thread/closed" -> {
            val threadId = params.string("threadId") ?: return threads to selected
            threads.map { summary ->
                if (summary.id == threadId) summary.copy(lifecycleLabel = "notLoaded") else summary
            } to selected
        }
        "turn/started", "turn/completed" -> {
            val updated = params["turn"]?.jsonObjectOrNull()?.toConversationTurn()
            val threadId = params.string("threadId")
            threads to selected?.replaceTurn(threadId, updated)
        }
        "item/started", "item/completed" -> {
            val threadId = params.string("threadId")
            val turnId = params.string("turnId")
            val item = params["item"]?.jsonObjectOrNull()?.toConversationItem()
            threads to selected?.replaceItem(threadId, turnId, item)
        }
        "item/agentMessage/delta" -> {
            val threadId = params.string("threadId")
            val turnId = params.string("turnId")
            val itemId = params.string("itemId")
            val delta = params.string("delta").orEmpty()
            threads to selected?.appendAgentDelta(threadId, turnId, itemId, delta)
        }
        "item/commandExecution/outputDelta" -> {
            val threadId = params.string("threadId")
            val turnId = params.string("turnId")
            val itemId = params.string("itemId")
            val delta = params.string("delta").orEmpty()
            threads to selected?.appendBodyDelta(threadId, turnId, itemId, delta)
        }
        else -> threads to selected
    }
}

private fun upsertSummary(current: List<ThreadSummary>, next: ThreadSummary): List<ThreadSummary> {
    val without = current.filterNot { it.id == next.id }
    return listOf(next) + without
}

private fun ConversationThread.replaceTurn(
    threadId: String?,
    turn: ConversationTurn?,
): ConversationThread {
    if (threadId != id || turn == null) return this
    val replaced = turns.any { it.id == turn.id }
    val nextTurns = if (replaced) turns.map { if (it.id == turn.id) turn else it } else turns + turn
    return copy(turns = nextTurns)
}

private fun ConversationThread.replaceItem(
    threadId: String?,
    turnId: String?,
    item: ConversationItem?,
): ConversationThread {
    if (threadId != id || turnId == null || item == null) return this
    return copy(turns = turns.upsertItem(turnId, item))
}

private fun ConversationThread.appendAgentDelta(
    threadId: String?,
    turnId: String?,
    itemId: String?,
    delta: String,
): ConversationThread {
    if (threadId != id || turnId == null || itemId == null || delta.isEmpty()) return this
    return appendBodyDelta(threadId, turnId, itemId, delta, title = "Assistant", type = "agentMessage")
}

private fun ConversationThread.appendBodyDelta(
    threadId: String?,
    turnId: String?,
    itemId: String?,
    delta: String,
    title: String = "Command",
    type: String = "commandExecution",
): ConversationThread {
    if (threadId != id || turnId == null || itemId == null || delta.isEmpty()) return this
    val fallback = ConversationItem(
        id = itemId,
        type = type,
        title = title,
        body = "",
        raw = buildJsonObject {
            put("id", JsonPrimitive(itemId))
            put("type", JsonPrimitive(type))
        },
    )
    return copy(turns = turns.upsertItem(turnId, fallback) { item ->
        item.copy(body = item.body + delta)
    })
}

private fun List<ConversationTurn>.upsertItem(
    turnId: String,
    item: ConversationItem,
    transformExisting: (ConversationItem) -> ConversationItem = { item },
): List<ConversationTurn> {
    val turnIndex = indexOfFirst { it.id == turnId }
    if (turnIndex == -1) {
        return this + ConversationTurn(
            id = turnId,
            status = "inProgress",
            items = listOf(transformExisting(item)),
            raw = JsonObject(emptyMap()),
        )
    }
    return map { turn ->
        if (turn.id != turnId) {
            turn
        } else {
            val hasItem = turn.items.any { it.id == item.id }
            val items = if (hasItem) {
                turn.items.map { if (it.id == item.id) transformExisting(it) else it }
            } else {
                turn.items + transformExisting(item)
            }
            turn.copy(items = items)
        }
    }
}
