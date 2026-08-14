package dev.morpheus.androidcompanion.state

import dev.morpheus.androidcompanion.rpc.RpcNotification
import dev.morpheus.androidcompanion.model.buildConversationCells
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
    fun itemStartedBeforeTurnStartedIsNotDroppedByEmptyTurnSnapshot() {
        val read = json.parseToJsonElement(
            """{"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[]}}""",
        )
        var selected = reduceThreadRead(read)
        val itemNotification = RpcNotification(
            "item/started",
            json.parseToJsonElement("""{"threadId":"t1","turnId":"turn-1","item":{"type":"agentMessage","id":"a1","text":"Hello"}}"""),
        )
        selected = applyNotification(emptyList(), selected, itemNotification).second
        val turnNotification = RpcNotification(
            "turn/started",
            json.parseToJsonElement("""{"threadId":"t1","turn":{"id":"turn-1","items":[],"itemsView":"full","status":"inProgress","error":null,"startedAt":3,"completedAt":null}}"""),
        )
        selected = applyNotification(emptyList(), selected, turnNotification).second

        assertEquals("Hello", selected?.turns?.first()?.items?.first()?.body)
    }

    @Test
    fun staleThreadReadPreservesLiveDeltaForSameItem() {
        val initial = reduceThreadRead(
            json.parseToJsonElement(
                """{"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[{"id":"turn-1","items":[{"type":"agentMessage","id":"a1","text":"Hello"}],"itemsView":"full","status":"inProgress","error":null,"startedAt":1,"completedAt":null}]}}""",
            ),
        )
        val delta = RpcNotification(
            "item/agentMessage/delta",
            json.parseToJsonElement("""{"threadId":"t1","turnId":"turn-1","itemId":"a1","delta":" world"}"""),
        )
        val live = applyNotification(emptyList(), initial, delta).second
        val staleRead = reduceThreadRead(
            json.parseToJsonElement(
                """{"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[{"id":"turn-1","items":[{"type":"agentMessage","id":"a1","text":"Hello"}],"itemsView":"full","status":"inProgress","error":null,"startedAt":1,"completedAt":null}]}}""",
            ),
        )
        val state = CompanionUiState(
            threads = listOf(initial!!.summary),
            selectedThreadId = "t1",
            selectedThread = live,
            isReadingThread = true,
        )

        val next = applySelectedThreadRead(state, "t1", staleRead)

        assertEquals("Hello world", next.selectedThread?.turns?.first()?.items?.first()?.body)
    }

    @Test
    fun itemCompletedDoesNotOverwriteLongerLiveDelta() {
        val read = json.parseToJsonElement(
            """{"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[{"id":"turn-1","items":[{"type":"agentMessage","id":"a1","text":"Hello"}],"itemsView":"full","status":"inProgress","error":null,"startedAt":1,"completedAt":null}]}}""",
        )
        var selected = reduceThreadRead(read)
        selected = applyNotification(
            emptyList(),
            selected,
            RpcNotification(
                "item/agentMessage/delta",
                json.parseToJsonElement("""{"threadId":"t1","turnId":"turn-1","itemId":"a1","delta":" world"}"""),
            ),
        ).second
        selected = applyNotification(
            emptyList(),
            selected,
            RpcNotification(
                "item/completed",
                json.parseToJsonElement("""{"threadId":"t1","turnId":"turn-1","item":{"type":"agentMessage","id":"a1","text":"Hello"}}"""),
            ),
        ).second

        assertEquals("Hello world", selected?.turns?.first()?.items?.first()?.body)
    }

    @Test
    fun largeAgentMessageSnapshotRetainsCompleteBodyInModel() {
        val longText = "a".repeat(30_000)
        val item = json.parseToJsonElement(
            """{"type":"agentMessage","id":"a1","text":"$longText"}""",
        ).jsonObject.toConversationItem()

        assertEquals(longText, item.body)
        assertTrueCompat(!item.body.contains("[truncated"))
        assertTrueCompat(!item.body.contains("showing latest text"))
    }

    @Test
    fun largeUserAndReasoningSnapshotsRetainCompleteBodyInModel() {
        val longText = "u".repeat(30_000)
        val reasoningText = "r".repeat(30_000)
        val userItem = json.parseToJsonElement(
            """{"type":"userMessage","id":"u1","content":[{"type":"text","text":"$longText"}]}""",
        ).jsonObject.toConversationItem()
        val reasoningItem = json.parseToJsonElement(
            """{"type":"reasoning","id":"r1","summary":["$reasoningText"]}""",
        ).jsonObject.toConversationItem()

        assertEquals(longText, userItem.body)
        assertTrueCompat(!userItem.body.contains("[truncated"))
        assertEquals(reasoningText, reasoningItem.body)
        assertTrueCompat(!reasoningItem.body.contains("[truncated"))
    }

    @Test
    fun repeatedAgentMessageDeltaRetainsCompleteBodyInModel() {
        val read = json.parseToJsonElement(
            """{"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[{"id":"turn-1","items":[{"type":"agentMessage","id":"a1","text":""}],"itemsView":"full","status":"inProgress","error":null,"startedAt":1,"completedAt":null}]}}""",
        )
        var selected = reduceThreadRead(read)
        repeat(40) { index ->
            val deltaNotification = RpcNotification(
                "item/agentMessage/delta",
                json.parseToJsonElement("""{"threadId":"t1","turnId":"turn-1","itemId":"a1","delta":"${index.toString().padStart(2, '0')}-${"x".repeat(500)}\n"}"""),
            )
            selected = applyNotification(emptyList(), selected, deltaNotification).second
        }

        val body = selected?.turns?.first()?.items?.first()?.body.orEmpty()
        assertEquals(20_160, body.length)
        assertTrueCompat(body.contains("00-"))
        assertTrueCompat(body.contains("39-"))
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
        assertTrueCompat(!presentation?.output.orEmpty().contains("[truncated"))
        assertTrueCompat(!presentation?.output.orEmpty().contains("showing latest output"))
    }

    @Test
    fun adjacentCommandExecutionsBuildOneToolCell() {
        val thread = reduceThreadRead(
            json.parseToJsonElement(
                """
                {"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[{"id":"turn-1","items":[{"type":"commandExecution","id":"cmd-1","command":"pwd","cwd":"/repo","status":"completed","aggregatedOutput":"one","exitCode":0},{"type":"commandExecution","id":"cmd-2","command":"ls","cwd":"/repo","status":"completed","aggregatedOutput":"two","exitCode":0}],"itemsView":"full","status":"completed","error":null,"startedAt":1,"completedAt":2}]}}
                """.trimIndent(),
            ),
        )

        val cells = thread!!.turns.buildConversationCells()

        assertEquals(1, cells.size)
        assertEquals("tool", cells.first().kind)
        assertEquals(listOf("cmd-1", "cmd-2"), cells.first().entries.map { it.id })
        assertEquals("one", cells.first().entries[0].toolPresentation?.output)
        assertEquals("two", cells.first().entries[1].toolPresentation?.output)
    }

    @Test
    fun commandNotificationsStaySeparateFromCommandCells() {
        val thread = reduceThreadRead(
            json.parseToJsonElement(
                """
                {"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[{"id":"turn-1","items":[{"type":"commandExecution","id":"cmd-1","command":"pwd","cwd":"/repo","status":"completed","aggregatedOutput":null,"exitCode":0},{"type":"commandExecutionNotification","id":"notice-1","commandItemId":"cmd-1","kind":"output","message":"changed","output":"changed","exitCode":null,"createdAtMs":1}],"itemsView":"full","status":"completed","error":null,"startedAt":1,"completedAt":2}]}}
                """.trimIndent(),
            ),
        )

        val cells = thread!!.turns.buildConversationCells()

        assertEquals(2, cells.size)
        assertEquals("command", cells[0].entries.first().toolPresentation?.toolCategory)
        assertEquals("commandNotification", cells[1].entries.first().toolPresentation?.toolCategory)
    }

    @Test
    fun commandCellDoesNotAbsorbFollowingMessage() {
        val thread = reduceThreadRead(
            json.parseToJsonElement(
                """
                {"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[{"id":"turn-1","items":[{"type":"commandExecution","id":"cmd-1","command":"pwd","cwd":"/repo","status":"completed","aggregatedOutput":null,"exitCode":0},{"type":"agentMessage","id":"a1","text":"Done"}],"itemsView":"full","status":"completed","error":null,"startedAt":1,"completedAt":2}]}}
                """.trimIndent(),
            ),
        )

        val cells = thread!!.turns.buildConversationCells()

        assertEquals(2, cells.size)
        assertEquals("tool", cells[0].kind)
        assertEquals("message", cells[1].kind)
        assertEquals("Done", cells[1].entries.first().body)
    }

    @Test
    fun adjacentAgentMessagesMergeButUserMessageCreatesBoundary() {
        val thread = reduceThreadRead(
            json.parseToJsonElement(
                """
                {"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[{"id":"turn-1","items":[{"type":"agentMessage","id":"a1","text":"One"},{"type":"agentMessage","id":"a2","text":"Two"},{"type":"userMessage","id":"u1","content":[{"type":"text","text":"Stop"}]},{"type":"agentMessage","id":"a3","text":"Three"}],"itemsView":"full","status":"completed","error":null,"startedAt":1,"completedAt":2}]}}
                """.trimIndent(),
            ),
        )

        val cells = thread!!.turns.buildConversationCells()

        assertEquals(3, cells.size)
        assertEquals(listOf("a1", "a2"), cells[0].entries.map { it.id })
        assertEquals(listOf("u1"), cells[1].entries.map { it.id })
        assertEquals(listOf("a3"), cells[2].entries.map { it.id })
    }

    @Test
    fun compactSnapshotDoesNotReviveMissingLiveTurns() {
        val liveThread = reduceThreadRead(
            json.parseToJsonElement(
                """{"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[{"id":"old-turn","items":[{"type":"agentMessage","id":"old-a1","text":"old"}],"itemsView":"full","status":"completed","error":null,"startedAt":1,"completedAt":2}]}}""",
            ),
        )
        val compactRead = reduceThreadRead(
            json.parseToJsonElement(
                """{"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":3,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[{"id":"compact-turn","items":[{"type":"contextCompaction","id":"compact-1"}],"itemsView":"full","status":"completed","error":null,"startedAt":3,"completedAt":3}]}}""",
            ),
        )
        val state = CompanionUiState(
            selectedThreadId = "t1",
            selectedThread = liveThread,
            isReadingThread = true,
        )

        val next = applySelectedThreadRead(state, "t1", compactRead)

        assertEquals(listOf("compact-turn"), next.selectedThread?.turns?.map { it.id })
    }

    @Test
    fun toolCategoriesMatchDesktopCommandEventAndMultiAgentBoundaries() {
        val thread = reduceThreadRead(
            json.parseToJsonElement(
                """
                {"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[{"id":"turn-1","items":[{"type":"commandExecution","id":"cmd-1","command":"pwd","cwd":"/repo","status":"completed","aggregatedOutput":null,"exitCode":0},{"type":"eventCommandCall","id":"monitor-1","command":"rtk test","label":"watch tests","status":"completed","output":{"ok":true}},{"type":"eventDrivenToolCall","id":"sub-1","tool":"process_exit_subscribe","arguments":{"label":"watch"},"status":"completed","output":{"ok":true}},{"type":"eventCommandEvent","id":"monitor-event-1","label":"watch tests","message":"watch completed"},{"type":"eventDrivenTool","id":"event-1","tool":"process_exit_subscribe","title":"Process exited","text":"watch completed"},{"type":"collabAgentToolCall","id":"agent-1","tool":"spawnAgent","status":"completed","arguments":{"target":"worker"},"output":{"ok":true}},{"type":"collabAgentMessage","id":"child-1","operation":"childCompletion","senderThreadId":"thread-2","senderPath":"/root/worker","recipientThreadId":"thread-1","recipientPath":"/root","otherRecipientPaths":[],"content":"done","triggerTurn":true},{"type":"collabAgentStatusUpdate","id":"status-1","senderThreadId":"thread-2","senderPath":"/root/worker","recipientThreadId":"thread-1","recipientPath":"/root","lifecycleStatus":{"path":"/root/worker","lifecycleStatus":{"type":"final","result":{"type":"completed","lastAgentMessage":"done"}},"message":"done"}}],"itemsView":"full","status":"completed","error":null,"startedAt":1,"completedAt":2}]}}
                """.trimIndent(),
            ),
        )

        val cells = thread!!.turns.buildConversationCells()

        assertEquals(
            listOf(
                "command",
                "eventDrivenSubscription",
                null,
                "eventDrivenEvent",
                "multiAgent",
                "childCompletion",
                "subagentNotification",
            ),
            cells.map { it.entries.first().toolPresentation?.toolCategory },
        )
        assertEquals(listOf("monitor-1", "sub-1"), cells[1].entries.map { it.id })
    }

    @Test
    fun adjacentOrdinaryMultiAgentToolsMergeButNotificationsStayStandalone() {
        val thread = reduceThreadRead(
            json.parseToJsonElement(
                """
                {"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[{"id":"turn-1","items":[{"type":"collabAgentToolCall","id":"spawn-1","tool":"spawnAgent","status":"completed","arguments":{"target":"worker"},"output":{"ok":true}},{"type":"collabAgentToolCall","id":"send-1","tool":"sendInput","status":"completed","arguments":{"target":"worker"},"output":{"ok":true}},{"type":"collabAgentMessage","id":"child-1","operation":"childCompletion","senderThreadId":"thread-2","senderPath":"/root/worker","recipientThreadId":"thread-1","recipientPath":"/root","otherRecipientPaths":[],"content":"done","triggerTurn":true},{"type":"collabAgentStatusUpdate","id":"status-1","senderThreadId":"thread-2","senderPath":"/root/worker","recipientThreadId":"thread-1","recipientPath":"/root","lifecycleStatus":{"path":"/root/worker","lifecycleStatus":{"type":"final","result":{"type":"completed","lastAgentMessage":"done"}},"message":"done"}},{"type":"collabAgentToolCall","id":"list-1","tool":"listAgents","status":"completed","arguments":{},"output":{"count":1}}],"itemsView":"full","status":"completed","error":null,"startedAt":1,"completedAt":2}]}}
                """.trimIndent(),
            ),
        )

        val cells = thread!!.turns.buildConversationCells()

        assertEquals(4, cells.size)
        assertEquals(listOf("spawn-1", "send-1"), cells[0].entries.map { it.id })
        assertEquals(listOf("child-1"), cells[1].entries.map { it.id })
        assertEquals("childCompletion", cells[1].entries.first().toolPresentation?.toolCategory)
        assertEquals(listOf("status-1"), cells[2].entries.map { it.id })
        assertEquals("subagentNotification", cells[2].entries.first().toolPresentation?.toolCategory)
        assertEquals(listOf("list-1"), cells[3].entries.map { it.id })
    }

    @Test
    fun collabAgentStatusUpdateLabelsExternalProviderCompletion() {
        val item = json.parseToJsonElement(
            """
            {
              "type": "collabAgentStatusUpdate",
              "id": "status-claude",
              "senderThreadId": "thread-2",
              "senderPath": "/root/claude",
              "recipientThreadId": "thread-1",
              "recipientPath": "/root",
              "lifecycleStatus": {
                "path": "/root/claude",
                "agentNickname": "claude_cli",
                "agentRole": "claude_cli",
                "lifecycleStatus": {
                  "type": "final",
                  "result": {"type": "completed", "lastAgentMessage": "done"}
                },
                "message": "done"
              }
            }
            """.trimIndent(),
        ).jsonObject.toConversationItem()

        val presentation = item.toolPresentation

        assertEquals("Claude Code /root/claude subagent completion", item.title)
        assertEquals("subagentNotification", presentation?.toolCategory)
        assertTrueCompat(presentation?.summary?.contains("Claude Code /root/claude") == true)
        assertTrueCompat(presentation?.details?.contains("Provider\nClaude Code") == true)
    }

    @Test
    fun commandExecutionOutputDeltaFallbackBuildsCommandToolCell() {
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
        val cells = updated?.turns.orEmpty().buildConversationCells()
        val presentation = item?.toolPresentation

        assertEquals("commandExecution", item?.type)
        assertEquals("command", presentation?.toolCategory)
        assertEquals("Output", presentation?.outputLabel)
        assertEquals("late output\n", presentation?.output)
        assertEquals("tool", cells.firstOrNull()?.kind)
    }

    @Test
    fun commandExecutionStartedAfterOutputDeltaPreservesFallbackOutput() {
        val read = json.parseToJsonElement(
            """{"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[]}}""",
        )
        var selected = reduceThreadRead(read)
        selected = applyNotification(
            emptyList(),
            selected,
            RpcNotification(
                "item/commandExecution/outputDelta",
                json.parseToJsonElement("""{"threadId":"t1","turnId":"turn-1","itemId":"cmd-1","delta":"late output\n"}"""),
            ),
        ).second
        selected = applyNotification(
            emptyList(),
            selected,
            RpcNotification(
                "item/started",
                json.parseToJsonElement(
                    """{"threadId":"t1","turnId":"turn-1","item":{"type":"commandExecution","id":"cmd-1","command":"rtk test","cwd":"/repo","status":"inProgress","aggregatedOutput":"","exitCode":null}}""",
                ),
            ),
        ).second

        val item = selected?.turns?.first()?.items?.first()
        val presentation = item?.toolPresentation

        assertEquals("command", presentation?.toolCategory)
        assertTrueCompat(presentation?.summary?.contains("rtk test") == true)
        assertEquals("late output\n", presentation?.output)
    }

    @Test
    fun commandExecutionCompletedAfterOutputDeltaPreservesFallbackOutput() {
        val read = json.parseToJsonElement(
            """{"thread":{"id":"t1","sessionId":"s1","preview":"","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":1,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":null,"skills":[],"tokenUsage":null,"contextUsage":null,"turns":[]}}""",
        )
        var selected = reduceThreadRead(read)
        selected = applyNotification(
            emptyList(),
            selected,
            RpcNotification(
                "item/commandExecution/outputDelta",
                json.parseToJsonElement("""{"threadId":"t1","turnId":"turn-1","itemId":"cmd-1","delta":"late output\n"}"""),
            ),
        ).second
        selected = applyNotification(
            emptyList(),
            selected,
            RpcNotification(
                "item/completed",
                json.parseToJsonElement(
                    """{"threadId":"t1","turnId":"turn-1","item":{"type":"commandExecution","id":"cmd-1","command":"rtk test","cwd":"/repo","status":"completed","aggregatedOutput":"","exitCode":0}}""",
                ),
            ),
        ).second

        val presentation = selected?.turns?.first()?.items?.first()?.toolPresentation

        assertEquals("completed", presentation?.status)
        assertTrueCompat(presentation?.summary?.contains("rtk test") == true)
        assertTrueCompat(presentation?.summary?.contains("exit 0") == true)
        assertTrueCompat(presentation?.details?.contains("Exit Code\n0") == true)
        assertEquals("late output\n", presentation?.output)
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
    fun dynamicToolCallProjectsContentItemsResult() {
        val item = json.parseToJsonElement(
            """
            {
              "type": "dynamicToolCall",
              "id": "tool-1",
              "tool": "search",
              "status": "completed",
              "arguments": {"query":"hello"},
              "contentItems": [{"type":"text","text":"dynamic result"}],
              "success": true,
              "durationMs": 12
            }
            """.trimIndent(),
        ).jsonObject.toConversationItem()

        val presentation = item.toolPresentation

        assertNotNull(presentation)
        assertEquals("external", presentation?.toolCategory)
        assertTrueCompat(presentation?.details?.contains("Content") == true)
        assertTrueCompat(presentation?.output?.contains("dynamic result") == true)
    }

    @Test
    fun dynamicToolCallNullContentItemsDoesNotRenderNullOutput() {
        val item = json.parseToJsonElement(
            """
            {
              "type": "dynamicToolCall",
              "id": "tool-1",
              "tool": "search",
              "status": "running",
              "arguments": {"query":"hello"},
              "contentItems": null
            }
            """.trimIndent(),
        ).jsonObject.toConversationItem()

        val presentation = item.toolPresentation

        assertEquals(true, presentation?.outputIsEmpty)
        assertNull(presentation?.output)
        assertTrueCompat(presentation?.details?.contains("Content") != true)
    }

    @Test
    fun mcpToolCallProjectsResultAndError() {
        val resultItem = json.parseToJsonElement(
            """
            {
              "type": "mcpToolCall",
              "id": "mcp-1",
              "tool": "read",
              "status": "completed",
              "arguments": {"path":"file"},
              "result": {"message":"mcp result"},
              "error": null
            }
            """.trimIndent(),
        ).jsonObject.toConversationItem()
        val errorItem = json.parseToJsonElement(
            """
            {
              "type": "mcpToolCall",
              "id": "mcp-2",
              "tool": "read",
              "status": "failed",
              "arguments": {"path":"file"},
              "result": null,
              "error": {"message":"mcp error"}
            }
            """.trimIndent(),
        ).jsonObject.toConversationItem()

        assertTrueCompat(resultItem.toolPresentation?.output?.contains("mcp result") == true)
        assertEquals("Result", resultItem.toolPresentation?.outputLabel)
        assertTrueCompat(errorItem.toolPresentation?.output?.contains("mcp error") == true)
        assertEquals("Error", errorItem.toolPresentation?.outputLabel)
    }

    @Test
    fun eventCommandEventCarriesBackendTruncationStateWithoutMarker() {
        val item = json.parseToJsonElement(
            """
            {
              "type": "eventCommandEvent",
              "id": "event-1",
              "subscriptionId": "sub-1",
              "kind": "output",
              "label": "watch tests",
              "command": "rtk test",
              "line": "real truncated word",
              "truncated": true,
              "createdAt": 1
            }
            """.trimIndent(),
        ).jsonObject.toConversationItem()

        assertEquals(true, item.bodyIsTruncated)
        assertEquals("real truncated word\nrtk test", item.body)
        assertTrueCompat(!item.body.contains("[truncated"))
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
    fun builtinToolCallLargeDetailsAreBoundedWithoutInjectedMarker() {
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
        assertTrueCompat(!details.contains("[truncated]"))
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
