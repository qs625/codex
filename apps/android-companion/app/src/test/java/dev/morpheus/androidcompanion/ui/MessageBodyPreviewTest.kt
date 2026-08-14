package dev.morpheus.androidcompanion.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class MessageBodyPreviewTest {
    @Test
    fun textAtLimitIsNotTruncated() {
        val text = "x".repeat(8_000)

        val preview = messageBodyPreview(text)

        assertEquals(text, preview.text)
        assertEquals(false, preview.isTruncated)
    }

    @Test
    fun textJustOverLimitIsMarkedTruncated() {
        val text = "a" + "b".repeat(8_000)

        val preview = messageBodyPreview(text)

        assertEquals(true, preview.isTruncated)
        assertTrue(preview.text.startsWith("[truncated 1 chars; showing latest text]"))
        assertTrue(preview.text.endsWith("b".repeat(8_000)))
    }
}
