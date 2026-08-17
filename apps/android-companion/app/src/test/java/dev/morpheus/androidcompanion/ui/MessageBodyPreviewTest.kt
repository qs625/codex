package dev.morpheus.androidcompanion.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class MessageBodyPreviewTest {
    @Test
    fun conversationPageRequiresSelectedThread() {
        assertEquals(
            CompanionPage.Threads,
            resolveConnectedPage(CompanionPage.Conversation, selectedThreadId = null),
        )
        assertEquals(
            CompanionPage.Conversation,
            resolveConnectedPage(CompanionPage.Conversation, selectedThreadId = "thread-1"),
        )
    }

    @Test
    fun connectedBackOnlyConsumesConversationDetail() {
        assertEquals(CompanionPage.Threads, connectedBackTarget(CompanionPage.Conversation))
        assertEquals(null, connectedBackTarget(CompanionPage.Threads))
    }

    @Test
    fun textAtLimitIsNotTruncated() {
        val text = "x".repeat(8_000)

        val preview = messageBodyPreview(text)

        assertEquals(text, preview.text)
        assertEquals(false, preview.isTruncated)
    }

    @Test
    fun textJustOverLimitUsesBoundedPreviewWithoutMarker() {
        val text = "a" + "b".repeat(8_000)

        val preview = messageBodyPreview(text)

        assertEquals(true, preview.isTruncated)
        assertEquals(8_000, preview.text.length)
        assertEquals(false, preview.text.contains("[truncated"))
        assertTrue(preview.text.endsWith("b".repeat(8_000)))
    }

    @Test
    fun expandedTextUsesCompleteBody() {
        val text = "a" + "b".repeat(8_000)

        assertEquals(8_000, visibleMessageBodyText(text, expanded = false).length)
        assertEquals(text, visibleMessageBodyText(text, expanded = true))
    }

    @Test
    fun realTruncatedWordIsPreserved() {
        val text = "real truncated content"

        val preview = messageBodyPreview(text)

        assertEquals("real truncated content", preview.text)
        assertEquals(false, preview.isTruncated)
    }

    @Test
    fun scrollTargetChangesWhenLastItemContentGrows() {
        val first = listOf(
            dev.morpheus.androidcompanion.model.ConversationCell(
                id = "a1",
                kind = "message",
                entries = listOf(
                    dev.morpheus.androidcompanion.model.ConversationItem(
                        id = "a1",
                        type = "agentMessage",
                        title = "Assistant",
                        body = "hi",
                    ),
                ),
            ),
        )
        val second = listOf(
            first.first().copy(
                entries = listOf(first.first().entries.first().copy(body = "hi there")),
            ),
        )

        assertTrue(conversationScrollTarget(first) != conversationScrollTarget(second))
    }

    @Test
    fun nearBottomAllowsOneItemThreshold() {
        assertEquals(true, isConversationNearBottom(lastVisibleIndex = 8, totalItemsCount = 10))
        assertEquals(false, isConversationNearBottom(lastVisibleIndex = 7, totalItemsCount = 10))
    }
}
