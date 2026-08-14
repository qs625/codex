package dev.morpheus.androidcompanion.state

import dev.morpheus.androidcompanion.rpc.RpcNotification
import dev.morpheus.androidcompanion.model.toConversationItem
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test

class ThreadReducerTest {
    private val json = Json { ignoreUnknownKeys = true }

    @Test
    fun threadReadSnapshotProjectsConversationItems() {
        val snapshot = json.parseToJsonElement(
            """
            {
              "thread": {
                "id": "t1",
                "sessionId": "s1",
                "preview": "hello",
                "ephemeral": false,
                "modelProvider": "openai",
                "createdAt": 1,
                "updatedAt": 2,
                "lifecycleStatus": "notLoaded",
                "path": null,
                "cwd": "/repo",
                "cliVersion": "0",
                "source": "appServer",
                "threadSource": "user",
                "agentNickname": null,
                "agentRole": null,
                "agentPath": "/root",
                "gitInfo": null,
                "name": "Demo",
                "skills": [],
                "tokenUsage": null,
                "contextUsage": null,
                "turns": [
                  {
                    "id": "turn-1",
                    "items": [
                      {"type":"userMessage","id":"u1","content":[{"type":"text","text":"Hi"}]},
                      {"type":"agentMessage","id":"a1","text":"Hello"}
                    ],
                    "itemsView": "full",
                    "status": "completed",
                    "error": null,
                    "startedAt": 1,
                    "completedAt": 2
                  }
                ]
              }
            }
            """.trimIndent(),
        )

        val thread = reduceThreadRead(snapshot)

        assertNotNull(thread)
        assertEquals("Demo", thread?.summary?.title)
        assertEquals("Hi", thread?.turns?.first()?.items?.first()?.body)
        assertEquals("Hello", thread?.turns?.first()?.items?.last()?.body)
    }

    @Test
    fun liveNotificationsMergeTurnItemAndDelta() {
        val read = json.parseToJsonElement(
            """{"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[]}}""",
        )
        var selected = reduceThreadRead(read)
        val turnNotification = RpcNotification(
            "turn/started",
            json.parseToJsonElement("""{"threadId":"t1","turn":{"id":"turn-1","items":[],"itemsView":"full","status":"inProgress","error":null,"startedAt":3,"completedAt":null}}"""),
        )
        selected = applyNotification(emptyList(), selected, turnNotification).second
        val itemNotification = RpcNotification(
            "item/started",
            json.parseToJsonElement("""{"threadId":"t1","turnId":"turn-1","item":{"type":"agentMessage","id":"a1","text":""}}"""),
        )
        selected = applyNotification(emptyList(), selected, itemNotification).second
        val deltaNotification = RpcNotification(
            "item/agentMessage/delta",
            json.parseToJsonElement("""{"threadId":"t1","turnId":"turn-1","itemId":"a1","delta":"Hello"}"""),
        )
        selected = applyNotification(emptyList(), selected, deltaNotification).second

        assertEquals("Hello", selected?.turns?.first()?.items?.first()?.body)
    }

    @Test
    fun structuredLifecycleStatusProjectsReadableLabels() {
        val waitingThread = reduceThreadRead(
            json.parseToJsonElement(
                """{"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":{"type":"waiting","reason":"command"},"path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[]}}""",
            ),
        )
        val completedThread = reduceThreadRead(
            json.parseToJsonElement(
                """{"thread":{"id":"t2","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":{"type":"final","result":{"type":"completed","lastAgentMessage":null}},"path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root/worker","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[]}}""",
            ),
        )

        assertEquals("Waiting on Event Tool", waitingThread?.summary?.lifecycleLabel)
        assertEquals("completed", completedThread?.summary?.lifecycleLabel)
    }

    @Test
    fun threadStatusChangedUpdatesListAndSelectedThreadLifecycle() {
        val selected = reduceThreadRead(
            json.parseToJsonElement(
                """{"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":{"type":"waiting","reason":"command"},"path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[]}}""",
            ),
        )
        val notification = RpcNotification(
            "thread/status/changed",
            json.parseToJsonElement(
                """{"threadId":"t1","lifecycleStatus":{"type":"final","result":{"type":"completed","lastAgentMessage":null}}}""",
            ),
        )

        val (threads, thread) = applyNotification(listOf(selected!!.summary), selected, notification)

        assertEquals("completed", threads.first().lifecycleLabel)
        assertEquals("completed", thread?.summary?.lifecycleLabel)
    }

    @Test
    fun staleThreadReadCannotDowngradeCompletedLifecycleStatus() {
        val completedThread = reduceThreadRead(
            json.parseToJsonElement(
                """{"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":2,"lifecycleStatus":{"type":"final","result":{"type":"completed","lastAgentMessage":null}},"path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[]}}""",
            ),
        )
        val staleWaitingThread = reduceThreadRead(
            json.parseToJsonElement(
                """{"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":{"type":"waiting","reason":"command"},"path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[]}}""",
            ),
        )
        val state = CompanionUiState(
            selectedThreadId = "t1",
            selectedThread = completedThread,
            isReadingThread = true,
        )

        val next = applySelectedThreadRead(state, "t1", staleWaitingThread)

        assertEquals("completed", next.selectedThread?.summary?.lifecycleLabel)
        assertEquals(false, next.isReadingThread)
    }

    @Test
    fun staleThreadReadUsesCompletedListLifecycleWhenSelectingThread() {
        val completedSummary = reduceThreadRead(
            json.parseToJsonElement(
                """{"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":2,"lifecycleStatus":{"type":"final","result":{"type":"completed","lastAgentMessage":null}},"path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[]}}""",
            ),
        )!!.summary
        val staleWaitingThread = reduceThreadRead(
            json.parseToJsonElement(
                """{"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":{"type":"waiting","reason":"command"},"path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[]}}""",
            ),
        )
        val state = CompanionUiState(
            threads = listOf(completedSummary),
            selectedThreadId = "t1",
            selectedThread = null,
            isReadingThread = true,
        )

        val next = applySelectedThreadRead(state, "t1", staleWaitingThread)

        assertEquals("completed", next.selectedThread?.summary?.lifecycleLabel)
        assertEquals(false, next.isReadingThread)
    }

    @Test
    fun staleThreadReadUsesCompletedListLifecycleOverWaitingSelectedThread() {
        val waitingThread = reduceThreadRead(
            json.parseToJsonElement(
                """{"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":{"type":"waiting","reason":"command"},"path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[]}}""",
            ),
        )
        val completedSummary = waitingThread!!.summary.copy(lifecycleLabel = "completed")
        val state = CompanionUiState(
            threads = listOf(completedSummary),
            selectedThreadId = "t1",
            selectedThread = waitingThread,
            isReadingThread = true,
        )

        val next = applySelectedThreadRead(state, "t1", waitingThread)

        assertEquals("completed", next.selectedThread?.summary?.lifecycleLabel)
        assertEquals(false, next.isReadingThread)
    }

    @Test
    fun unknownItemHasReadableFallback() {
        val item = json.parseToJsonElement("""{"type":"futureItem","id":"x1","payload":{"ok":true,"large":"${"x".repeat(800)}"}}""")
            .jsonObject
            .toConversationItem()

        assertEquals("futureItem", item.title)
        assertTrueCompat(item.body.contains("payload"))
        assertTrueCompat(item.body.length < 430)
        assertNull(item.toolPresentation)
    }

    @Test
    fun commandExecutionProjectsCollapsedToolPresentation() {
        val item = json.parseToJsonElement(
            """
            {
              "type": "commandExecution",
              "id": "cmd-1",
              "command": "rtk cargo test -p app-server",
              "cwd": "/repo/codex-rs",
              "status": "completed",
              "aggregatedOutput": "ok\n",
              "exitCode": 0,
              "durationMs": 42
            }
            """.trimIndent(),
        ).jsonObject.toConversationItem()

        val presentation = item.toolPresentation
        assertNotNull(presentation)
        assertEquals("Command", item.title)
        assertTrueCompat(item.body.contains("rtk cargo test"))
        assertEquals("completed", presentation?.status)
        assertEquals("Output", presentation?.outputLabel)
        assertEquals("ok\n", presentation?.output)
        assertTrueCompat(presentation?.details?.contains("Cwd\n/repo/codex-rs") == true)
    }

    @Test
    fun commandExecutionOutputDeltaUpdatesToolOutput() {
        val read = json.parseToJsonElement(
            """{"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[{"id":"turn-1","items":[{"type":"commandExecution","id":"cmd-1","command":"rtk test","cwd":"/repo","status":"inProgress","aggregatedOutput":"start\n","exitCode":null}],"itemsView":"full","status":"inProgress","error":null,"startedAt":1,"completedAt":null}]}}""",
        )
        val selected = reduceThreadRead(read)
        val deltaNotification = RpcNotification(
            "item/commandExecution/outputDelta",
            json.parseToJsonElement("""{"threadId":"t1","turnId":"turn-1","itemId":"cmd-1","delta":"more\n"}"""),
        )

        val updated = applyNotification(emptyList(), selected, deltaNotification).second
        val presentation = updated?.turns?.first()?.items?.first()?.toolPresentation

        assertEquals("start\nmore\n", presentation?.output)
        assertEquals(false, presentation?.outputIsEmpty)
    }

    @Test
    fun commandExecutionLargeOutputIsBounded() {
        val longOutput = "o".repeat(30_000)
        val item = json.parseToJsonElement(
            """
            {
              "type": "commandExecution",
              "id": "cmd-1",
              "command": "rtk long",
              "cwd": "/repo",
              "status": "completed",
              "aggregatedOutput": "$longOutput",
              "exitCode": 0
            }
            """.trimIndent(),
        ).jsonObject.toConversationItem()

        val presentation = item.toolPresentation
        assertNotNull(presentation)
        assertEquals(true, presentation?.outputIsTruncated)
        assertTrueCompat(presentation?.output.orEmpty().length < 13_000)
        assertTrueCompat(presentation?.output.orEmpty().contains("showing latest output"))
    }

    @Test
    fun commandExecutionOutputDeltaFallbackKeepsBodyVisible() {
        val read = json.parseToJsonElement(
            """{"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[]}}""",
        )
        val selected = reduceThreadRead(read)
        val deltaNotification = RpcNotification(
            "item/commandExecution/outputDelta",
            json.parseToJsonElement("""{"threadId":"t1","turnId":"turn-1","itemId":"cmd-1","delta":"late output\n"}"""),
        )

        val updated = applyNotification(emptyList(), selected, deltaNotification).second
        val item = updated?.turns?.first()?.items?.first()

        assertEquals("late output\n", item?.body)
        assertNull(item?.toolPresentation)
    }

    @Test
    fun repeatedCommandOutputDeltaStaysBounded() {
        val read = json.parseToJsonElement(
            """{"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[{"id":"turn-1","items":[{"type":"commandExecution","id":"cmd-1","command":"rtk test","cwd":"/repo","status":"inProgress","aggregatedOutput":"","exitCode":null}],"itemsView":"full","status":"inProgress","error":null,"startedAt":1,"completedAt":null}]}}""",
        )
        var selected = reduceThreadRead(read)
        repeat(40) { index ->
            val deltaNotification = RpcNotification(
                "item/commandExecution/outputDelta",
                json.parseToJsonElement("""{"threadId":"t1","turnId":"turn-1","itemId":"cmd-1","delta":"${index.toString().padStart(2, '0')}-${"x".repeat(500)}\n"}"""),
            )
            selected = applyNotification(emptyList(), selected, deltaNotification).second
        }

        val presentation = selected?.turns?.first()?.items?.first()?.toolPresentation
        assertEquals(true, presentation?.outputIsTruncated)
        assertTrueCompat(presentation?.output.orEmpty().length < 13_000)
        assertTrueCompat(presentation?.output.orEmpty().contains("39-"))
    }

    @Test
    fun commandNotificationProjectsToolOutput() {
        val item = json.parseToJsonElement(
            """
            {
              "type": "commandExecutionNotification",
              "id": "notice-1",
              "commandItemId": "cmd-1",
              "kind": "exit",
              "message": "Command exited",
              "output": "done\n",
              "exitCode": 0,
              "createdAtMs": 10
            }
            """.trimIndent(),
        ).jsonObject.toConversationItem()

        val presentation = item.toolPresentation
        assertNotNull(presentation)
        assertEquals("completed", presentation?.status)
        assertTrueCompat(presentation?.summary?.contains("exit 0") == true)
        assertEquals("done\n", presentation?.output)
    }

    @Test
    fun builtinToolCallProjectsToolPresentation() {
        val item = json.parseToJsonElement(
            """
            {
              "type": "builtinToolCall",
              "id": "tool-1",
              "tool": "poll_event",
              "status": "completed",
              "arguments": {"n":0,"b":true,"nil":null,"s":"true","q\"key":"a\"b"},
              "output": {"sourceHint":"child_completion"}
            }
            """.trimIndent(),
        ).jsonObject.toConversationItem()

        val presentation = item.toolPresentation
        assertNotNull(presentation)
        assertEquals("poll_event", item.title)
        assertEquals("completed", presentation?.status)
        assertTrueCompat(presentation?.details?.contains("Arguments") == true)
        assertTrueCompat(presentation?.details?.contains("\"n\":0") == true)
        assertTrueCompat(presentation?.details?.contains("\"b\":true") == true)
        assertTrueCompat(presentation?.details?.contains("\"nil\":null") == true)
        assertTrueCompat(presentation?.details?.contains("\"s\":\"true\"") == true)
        assertTrueCompat(presentation?.details?.contains("\"q\\\"key\":\"a\\\"b\"") == true)
    }

    @Test
    fun itemNotificationCanBeProjectedBeforeApplyingToLatestState() {
        val notification = RpcNotification(
            "item/completed",
            json.parseToJsonElement(
                """
                {
                  "threadId": "t1",
                  "turnId": "turn-1",
                  "item": {
                    "type": "builtinToolCall",
                    "id": "tool-1",
                    "tool": "read_agent",
                    "status": "completed",
                    "arguments": {"target":"worker"},
                    "output": {"message":"${"x".repeat(30_000)}"}
                  }
                }
                """.trimIndent(),
            ),
        )

        val projected = projectNotification(notification)

        assertTrueCompat(projected is ProjectedNotification.ItemUpdated)
        val item = (projected as ProjectedNotification.ItemUpdated).item
        assertEquals(true, item?.toolPresentation?.detailsIsTruncated)
        assertTrueCompat(item?.toolPresentation?.details.orEmpty().length < 9_000)
    }

    @Test
    fun builtinToolCallLargeDetailsAreBoundedAndMarkedTruncated() {
        val longOutput = "x".repeat(30_000)
        val item = json.parseToJsonElement(
            """
            {
              "type": "builtinToolCall",
              "id": "tool-1",
              "tool": "read_agent",
              "status": "completed",
              "arguments": {"target":"worker"},
              "output": {"message":"$longOutput"}
            }
            """.trimIndent(),
        ).jsonObject.toConversationItem()

        val presentation = item.toolPresentation
        val details = presentation?.details.orEmpty()
        assertEquals(true, presentation?.detailsIsTruncated)
        assertTrueCompat(details.length < 9_000)
        assertTrueCompat(details.contains("[truncated]"))
    }

    @Test
    fun staleThreadReadCannotReplaceCurrentSelection() {
        val staleThread = reduceThreadRead(
            json.parseToJsonElement(
                """{"thread":{"id":"a","sessionId":"s","preview":"A","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root/a","gitInfo":null,"name":"A","skills":[],"tokenUsage":null,"contextUsage":null,"turns":[]}}""",
            ),
        )
        val currentThread = reduceThreadRead(
            json.parseToJsonElement(
                """{"thread":{"id":"b","sessionId":"s","preview":"B","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":2,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root/b","gitInfo":null,"name":"B","skills":[],"tokenUsage":null,"contextUsage":null,"turns":[]}}""",
            ),
        )
        val state = CompanionUiState(
            selectedThreadId = "b",
            selectedThread = currentThread,
            isReadingThread = true,
        )

        val next = applySelectedThreadRead(state, "a", staleThread)

        assertEquals("b", next.selectedThread?.id)
        assertEquals(true, next.isReadingThread)
    }
}

private fun assertTrueCompat(value: Boolean) {
    org.junit.Assert.assertTrue(value)
}
