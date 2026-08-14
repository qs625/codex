package dev.morpheus.androidcompanion.state

import dev.morpheus.androidcompanion.model.ConversationItem
import dev.morpheus.androidcompanion.model.ConversationThread
import dev.morpheus.androidcompanion.model.ConversationTurn
import dev.morpheus.androidcompanion.model.ThreadSummary
import dev.morpheus.androidcompanion.model.appendBodyDeltaBounded
import dev.morpheus.androidcompanion.model.appendToolOutputDelta
import dev.morpheus.androidcompanion.model.jsonObjectOrNull
import dev.morpheus.androidcompanion.model.lifecycleStatusLabel
import dev.morpheus.androidcompanion.model.string
import dev.morpheus.androidcompanion.model.toConversationItem
import dev.morpheus.androidcompanion.model.toConversationThread
import dev.morpheus.androidcompanion.model.toConversationTurn
import dev.morpheus.androidcompanion.model.toThreadSummary
import dev.morpheus.androidcompanion.rpc.RpcNotification
import kotlinx.serialization.json.JsonElement

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
        val selectedThread = if (thread != null) {
            thread.withTerminalLifecycleLabelPreserved(
                listOfNotNull(
                    current.selectedThread
                        ?.takeIf { it.id == thread.id }
                        ?.summary
                        ?.lifecycleLabel,
                    current.threads
                        .firstOrNull { it.id == thread.id }
                        ?.lifecycleLabel,
                ).firstOrNull { it.isTerminalLifecycleLabel() },
            )
        } else {
            thread
        }
        current.copy(selectedThread = selectedThread, isReadingThread = false)
    } else {
        current
    }
}

fun applyNotification(
    threads: List<ThreadSummary>,
    selected: ConversationThread?,
    notification: RpcNotification,
): Pair<List<ThreadSummary>, ConversationThread?> {
    val projected = projectNotification(notification) ?: return threads to selected
    return applyProjectedNotification(threads, selected, projected)
}

fun projectNotification(notification: RpcNotification): ProjectedNotification? {
    val params = notification.params?.jsonObjectOrNull() ?: return null
    return when (notification.method) {
        "thread/started" -> {
            val thread = params["thread"]?.jsonObjectOrNull()?.toThreadSummary()
                ?: return null
            ProjectedNotification.ThreadStarted(thread)
        }
        "thread/name/updated" -> {
            val threadId = params.string("threadId") ?: return null
            val name = params.string("threadName")
            ProjectedNotification.ThreadNameUpdated(threadId, name)
        }
        "thread/status/changed" -> {
            val threadId = params.string("threadId") ?: return null
            val status = lifecycleStatusLabel(params["lifecycleStatus"])
            ProjectedNotification.ThreadStatusChanged(threadId, status)
        }
        "thread/archived" -> {
            val threadId = params.string("threadId") ?: return null
            ProjectedNotification.ThreadArchived(threadId)
        }
        "thread/closed" -> {
            val threadId = params.string("threadId") ?: return null
            ProjectedNotification.ThreadClosed(threadId)
        }
        "turn/started", "turn/completed" -> {
            val updated = params["turn"]?.jsonObjectOrNull()?.toConversationTurn()
            val threadId = params.string("threadId")
            ProjectedNotification.TurnUpdated(threadId, updated)
        }
        "item/started", "item/completed" -> {
            val threadId = params.string("threadId")
            val turnId = params.string("turnId")
            val item = params["item"]?.jsonObjectOrNull()?.toConversationItem()
            ProjectedNotification.ItemUpdated(threadId, turnId, item)
        }
        "item/agentMessage/delta" -> {
            val threadId = params.string("threadId")
            val turnId = params.string("turnId")
            val itemId = params.string("itemId")
            val delta = params.string("delta").orEmpty()
            ProjectedNotification.AgentMessageDelta(threadId, turnId, itemId, delta)
        }
        "item/commandExecution/outputDelta" -> {
            val threadId = params.string("threadId")
            val turnId = params.string("turnId")
            val itemId = params.string("itemId")
            val delta = params.string("delta").orEmpty()
            ProjectedNotification.CommandOutputDelta(threadId, turnId, itemId, delta)
        }
        else -> null
    }
}

fun applyProjectedNotification(
    threads: List<ThreadSummary>,
    selected: ConversationThread?,
    notification: ProjectedNotification,
): Pair<List<ThreadSummary>, ConversationThread?> {
    return when (notification) {
        is ProjectedNotification.ThreadStarted ->
            upsertSummary(threads, notification.thread) to selected
        is ProjectedNotification.ThreadNameUpdated ->
            threads.map { summary ->
                if (summary.id == notification.threadId) {
                    summary.copy(
                        title = notification.name ?: summary.preview.ifBlank { summary.id.take(12) },
                    )
                } else {
                    summary
                }
            } to selected
        is ProjectedNotification.ThreadStatusChanged ->
            threads.map { summary ->
                if (summary.id == notification.threadId) {
                    summary.copy(lifecycleLabel = notification.status)
                } else {
                    summary
                }
            } to selected?.withLifecycleLabel(notification.threadId, notification.status)
        is ProjectedNotification.ThreadArchived ->
            threads.filterNot { it.id == notification.threadId } to
                selected?.takeUnless { it.id == notification.threadId }
        is ProjectedNotification.ThreadClosed ->
            threads.map { summary ->
                if (summary.id == notification.threadId) {
                    summary.copy(lifecycleLabel = "notLoaded")
                } else {
                    summary
                }
            } to selected
        is ProjectedNotification.TurnUpdated ->
            threads to selected?.replaceTurn(notification.threadId, notification.turn)
        is ProjectedNotification.ItemUpdated ->
            threads to selected?.replaceItem(
                notification.threadId,
                notification.turnId,
                notification.item,
            )
        is ProjectedNotification.AgentMessageDelta ->
            threads to selected?.appendAgentDelta(
                notification.threadId,
                notification.turnId,
                notification.itemId,
                notification.delta,
            )
        is ProjectedNotification.CommandOutputDelta ->
            threads to selected?.appendCommandOutputDelta(
                notification.threadId,
                notification.turnId,
                notification.itemId,
                notification.delta,
            )
    }
}

sealed interface ProjectedNotification {
    data class ThreadStarted(val thread: ThreadSummary) : ProjectedNotification
    data class ThreadNameUpdated(val threadId: String, val name: String?) : ProjectedNotification
    data class ThreadStatusChanged(val threadId: String, val status: String) : ProjectedNotification
    data class ThreadArchived(val threadId: String) : ProjectedNotification
    data class ThreadClosed(val threadId: String) : ProjectedNotification
    data class TurnUpdated(
        val threadId: String?,
        val turn: ConversationTurn?,
    ) : ProjectedNotification
    data class ItemUpdated(
        val threadId: String?,
        val turnId: String?,
        val item: ConversationItem?,
    ) : ProjectedNotification
    data class AgentMessageDelta(
        val threadId: String?,
        val turnId: String?,
        val itemId: String?,
        val delta: String,
    ) : ProjectedNotification
    data class CommandOutputDelta(
        val threadId: String?,
        val turnId: String?,
        val itemId: String?,
        val delta: String,
    ) : ProjectedNotification
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

private fun ConversationThread.withLifecycleLabel(
    threadId: String,
    status: String,
): ConversationThread {
    return if (id == threadId) {
        copy(summary = summary.copy(lifecycleLabel = status))
    } else {
        this
    }
}

private fun ConversationThread.withTerminalLifecycleLabelPreserved(
    currentLabel: String?,
): ConversationThread {
    return if (
        currentLabel != null &&
        currentLabel.isTerminalLifecycleLabel() &&
        !summary.lifecycleLabel.isTerminalLifecycleLabel()
    ) {
        copy(summary = summary.copy(lifecycleLabel = currentLabel))
    } else {
        this
    }
}

private fun String.isTerminalLifecycleLabel(): Boolean {
    return when (this) {
        "complete", "completed", "errored", "shutdown", "interrupted" -> true
        else -> false
    }
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
    )
    return copy(turns = turns.upsertItem(turnId, fallback) { item ->
        item.copy(body = appendBodyDeltaBounded(item.body, delta))
    })
}

private fun ConversationThread.appendCommandOutputDelta(
    threadId: String?,
    turnId: String?,
    itemId: String?,
    delta: String,
): ConversationThread {
    if (threadId != id || turnId == null || itemId == null || delta.isEmpty()) return this
    val fallback = ConversationItem(
        id = itemId,
        type = "commandExecution",
        title = "Command",
        body = "",
    )
    return copy(turns = turns.upsertItem(turnId, fallback) { item ->
        item.appendToolOutputDelta(delta)
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
