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
    fun unknownItemHasReadableFallback() {
        val item = json.parseToJsonElement("""{"type":"futureItem","id":"x1","payload":{"ok":true}}""")
            .jsonObject
            .toConversationItem()

        assertEquals("futureItem", item.title)
        assertTrueCompat(item.body.contains("payload"))
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
              "arguments": {},
              "output": {"sourceHint":"child_completion"}
            }
            """.trimIndent(),
        ).jsonObject.toConversationItem()

        val presentation = item.toolPresentation
        assertNotNull(presentation)
        assertEquals("poll_event", item.title)
        assertEquals("completed", presentation?.status)
        assertTrueCompat(presentation?.details?.contains("Arguments") == true)
    }

    @Test
    fun builtinToolCallDetailsDoNotTruncateLargeOutput() {
        val longOutput = "x".repeat(520)
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

        val details = item.toolPresentation?.details.orEmpty()
        assertTrueCompat(details.contains(longOutput))
        assertTrueCompat(!details.contains("..."))
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
