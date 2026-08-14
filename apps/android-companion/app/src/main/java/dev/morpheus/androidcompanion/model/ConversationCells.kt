package dev.morpheus.androidcompanion.model

data class ConversationCell(
    val id: String,
    val kind: String,
    val entries: List<ConversationItem>,
)

fun List<ConversationTurn>.buildConversationCells(): List<ConversationCell> {
    val entries = flatMap { turn ->
        turn.items.map { item ->
            ConversationCellEntry(
                turnId = turn.id,
                item = item,
                kind = item.conversationCellKind(),
                toolCategory = item.toolPresentation?.toolCategory,
            )
        }
    }
    val cells = mutableListOf<ConversationCellEntryGroup>()
    var index = 0
    while (index < entries.size) {
        val group = ConversationCellEntryGroup(
            kind = entries[index].kind,
            entries = mutableListOf(entries[index]),
        )
        index += 1
        while (index < entries.size && group.shouldMerge(entries[index])) {
            group.entries += entries[index]
            index += 1
        }
        cells += group
    }
    return cells.map { group ->
        ConversationCell(
            id = group.entries.first().item.id,
            kind = group.kind,
            entries = group.entries.map { it.item },
        )
    }
}

private data class ConversationCellEntry(
    val turnId: String,
    val item: ConversationItem,
    val kind: String,
    val toolCategory: String?,
)

private data class ConversationCellEntryGroup(
    val kind: String,
    val entries: MutableList<ConversationCellEntry>,
) {
    fun shouldMerge(next: ConversationCellEntry): Boolean {
        val previous = entries.lastOrNull() ?: return false
        if (kind == "tool" && next.kind == "tool") {
            if (previous.turnId != next.turnId) return false
            if (previous.isStandaloneToolNotification() || next.isStandaloneToolNotification()) {
                return false
            }
            return previous.toolCategory == next.toolCategory
        }
        if (
            kind == "message" &&
            next.kind == "message" &&
            previous.item.type == "agentMessage" &&
            next.item.type == "agentMessage"
        ) {
            return previous.turnId == next.turnId
        }
        return false
    }
}

private fun ConversationCellEntry.isStandaloneToolNotification(): Boolean {
    return toolCategory == "commandNotification" ||
        toolCategory == "childCompletion" ||
        toolCategory == "subagentNotification"
}

private fun ConversationItem.conversationCellKind(): String {
    if (toolPresentation != null) return "tool"
    return when (type) {
        "reasoning", "plan", "contextCompaction", "eventCommandEvent" -> "event"
        else -> "message"
    }
}
