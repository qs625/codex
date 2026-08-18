package dev.morpheus.androidcompanion.state

import dev.morpheus.androidcompanion.rpc.AppServerConnection
import dev.morpheus.androidcompanion.rpc.RpcConnectionEvent
import dev.morpheus.androidcompanion.rpc.RpcConnectionException
import dev.morpheus.androidcompanion.rpc.RpcNotification
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestDispatcher
import kotlinx.coroutines.test.TestCoroutineScheduler
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TestWatcher
import org.junit.runner.Description

@OptIn(ExperimentalCoroutinesApi::class)
class CompanionViewModelTest {
    @get:Rule
    val mainDispatcherRule = MainDispatcherRule()

    @Test
    fun loadsSavedConnectionIntoForm() {
        val viewModel = CompanionViewModel(
            settingsStore = InMemoryConnectionSettingsStore(
                ConnectionSettings("wss://public.example/root-worker", "secret"),
            ),
        )

        assertEquals("wss://public.example/root-worker", viewModel.state.value.connectionEndpoint)
        assertEquals("secret", viewModel.state.value.connectionToken)
    }

    @Test
    fun successfulConnectPersistsLastKnownGoodConnection() = runTest(mainDispatcherRule.scheduler) {
        val store = InMemoryConnectionSettingsStore()
        val clients = mutableListOf<FakeConnection>()
        val viewModel = CompanionViewModel(
            settingsStore = store,
            clientFactory = { _, _ -> FakeConnection().also { clients.add(it) } },
        )
        try {
            viewModel.connect(" wss://public.example/root-worker ", " token ")
            advanceUntilIdle()

            assertEquals(ConnectionSettings("wss://public.example/root-worker", "token"), store.saved)
            assertTrue(viewModel.state.value.connection is ConnectionState.Connected)
            clients.single().awaitRequests("thread/list", "thread/read", "thread/resume")
            assertEquals(listOf("thread/list", "thread/read", "thread/resume"), clients.single().requests)
        } finally {
            viewModel.disconnect()
            advanceUntilIdle()
        }
    }

    @Test
    fun failedConnectDoesNotOverwritePreviousSavedConnection() = runTest(mainDispatcherRule.scheduler) {
        val store = InMemoryConnectionSettingsStore(
            ConnectionSettings("wss://public.example/root-worker", "secret"),
        )
        val viewModel = CompanionViewModel(
            settingsStore = store,
            clientFactory = { _, _ -> FakeConnection(connectError = RpcConnectionException("boom")) },
        )

        viewModel.connect("wss://bad.example/root-worker", "bad-token")
        advanceUntilIdle()

        assertEquals(ConnectionSettings("wss://public.example/root-worker", "secret"), store.saved)
        assertTrue(viewModel.state.value.connection is ConnectionState.Failed)
    }

    @Test
    fun manualDisconnectDoesNotReconnectClosedSocket() = runTest(mainDispatcherRule.scheduler) {
        val store = InMemoryConnectionSettingsStore()
        val clients = mutableListOf<FakeConnection>()
        val viewModel = CompanionViewModel(
            settingsStore = store,
            clientFactory = { _, _ -> FakeConnection().also { clients.add(it) } },
        )
        viewModel.connect("wss://public.example/root-worker", "token")
        advanceUntilIdle()
        val firstClient = clients.single()

        viewModel.disconnect()
        firstClient.emitFailure()
        advanceTimeBy(5_000)
        advanceUntilIdle()

        assertEquals(1, clients.size)
        assertEquals(ConnectionState.Disconnected, viewModel.state.value.connection)
    }

    @Test
    fun socketFailureReconnectsWithSavedConnectionAndRestoresSelectedThread() = runTest(mainDispatcherRule.scheduler) {
        val store = InMemoryConnectionSettingsStore()
        val clients = mutableListOf<FakeConnection>()
        val viewModel = CompanionViewModel(
            settingsStore = store,
            clientFactory = { endpoint, token ->
                FakeConnection(endpoint = endpoint, token = token).also { clients.add(it) }
            },
        )
        try {
            viewModel.connect("wss://public.example/root-worker", "token")
            advanceUntilIdle()
            clients.first().awaitRequests("thread/list", "thread/read", "thread/resume")
            clients.first().awaitEventSubscriber()
            assertEquals("t1", viewModel.state.value.selectedThreadId)

            clients.first().emitFailure()
            viewModel.awaitConnection<ConnectionState.Reconnecting>()
            advanceTimeBy(1_000)
            advanceUntilIdle()

            assertEquals(2, clients.size)
            assertEquals("wss://public.example/root-worker", clients.last().endpoint)
            assertEquals("token", clients.last().token)
            clients.last().awaitRequests("thread/list", "thread/read", "thread/resume")
            assertTrue(viewModel.state.value.connection is ConnectionState.Connected)
            assertEquals("t1", viewModel.state.value.selectedThreadId)
            assertEquals(listOf("thread/list", "thread/read", "thread/resume"), clients.last().requests)
        } finally {
            viewModel.disconnect()
            advanceUntilIdle()
        }
    }

    @Test
    fun staleSlowConnectCannotOverwriteNewerSuccessfulConnection() = runTest(mainDispatcherRule.scheduler) {
        val store = InMemoryConnectionSettingsStore()
        val firstConnectGate = CompletableDeferred<Unit>()
        val first = FakeConnection(
            endpoint = "wss://old.example/root-worker",
            token = "old-token",
            connectGate = firstConnectGate,
        )
        val second = FakeConnection(
            endpoint = "wss://new.example/root-worker",
            token = "new-token",
        )
        val clients = ArrayDeque(listOf(first, second))
        val viewModel = CompanionViewModel(
            settingsStore = store,
            clientFactory = { _, _ -> clients.removeFirst() },
        )
        try {
            viewModel.connect("wss://old.example/root-worker", "old-token")
            advanceUntilIdle()
            viewModel.connect("wss://new.example/root-worker", "new-token")
            advanceUntilIdle()
            firstConnectGate.complete(Unit)
            advanceUntilIdle()

            assertEquals(ConnectionSettings("wss://new.example/root-worker", "new-token"), store.saved)
            assertEquals(ConnectionState.Connected("wss://new.example/root-worker"), viewModel.state.value.connection)
            assertTrue(first.closed)
        } finally {
            viewModel.disconnect()
            advanceUntilIdle()
        }
    }

    @Test
    fun disconnectWhileConnectIsInFlightCannotRestoreConnection() = runTest(mainDispatcherRule.scheduler) {
        val store = InMemoryConnectionSettingsStore()
        val connectGate = CompletableDeferred<Unit>()
        val client = FakeConnection(connectGate = connectGate)
        val viewModel = CompanionViewModel(
            settingsStore = store,
            clientFactory = { _, _ -> client },
        )

        viewModel.connect("wss://public.example/root-worker", "token")
        advanceUntilIdle()
        viewModel.disconnect()
        connectGate.complete(Unit)
        advanceUntilIdle()

        assertEquals(null, store.saved)
        assertEquals(ConnectionState.Disconnected, viewModel.state.value.connection)
        assertTrue(client.closed)
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class MainDispatcherRule(
    val scheduler: TestCoroutineScheduler = TestCoroutineScheduler(),
    private val dispatcher: TestDispatcher = StandardTestDispatcher(scheduler),
) : TestWatcher() {
    override fun starting(description: Description) {
        Dispatchers.setMain(dispatcher)
    }

    override fun finished(description: Description) {
        Dispatchers.resetMain()
    }
}

private suspend fun FakeConnection.awaitRequests(vararg methods: String) {
    val expected = methods.toList()
    try {
        withTimeout(1_000) {
            while (requests != expected) {
                delay(1)
            }
        }
    } catch (error: TimeoutCancellationException) {
        assertEquals(expected, requests)
    }
}

private suspend inline fun <reified T : ConnectionState> CompanionViewModel.awaitConnection() {
    try {
        withTimeout(1_000) {
            while (state.value.connection !is T) {
                delay(1)
            }
        }
    } catch (error: TimeoutCancellationException) {
        assertTrue("Expected ${T::class.simpleName}, got ${state.value.connection}", state.value.connection is T)
    }
}

private class InMemoryConnectionSettingsStore(
    initial: ConnectionSettings? = null,
) : ConnectionSettingsStore {
    var saved: ConnectionSettings? = initial

    override fun load(): ConnectionSettings? = saved

    override fun save(settings: ConnectionSettings) {
        saved = settings
    }
}

private class FakeConnection(
    val endpoint: String = "wss://public.example/root-worker",
    val token: String? = "token",
    private val connectError: Throwable? = null,
    private val connectGate: CompletableDeferred<Unit>? = null,
) : AppServerConnection {
    private val notifications = MutableSharedFlow<RpcNotification>()
    private val events = MutableSharedFlow<RpcConnectionEvent>(extraBufferCapacity = 8)
    val requests = mutableListOf<String>()
    var closed = false

    override val serverNotifications: SharedFlow<RpcNotification> = notifications.asSharedFlow()
    override val connectionEvents: SharedFlow<RpcConnectionEvent> = events.asSharedFlow()

    override suspend fun connect(timeoutMs: Long) {
        connectGate?.await()
        connectError?.let { throw it }
    }

    override suspend fun request(
        method: String,
        params: JsonElement,
        timeoutMs: Long,
    ): JsonElement {
        requests.add(method)
        return when (method) {
            "thread/list" -> json.parseToJsonElement(
                """{"data":[{"id":"t1","sessionId":"s1","preview":"hello","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":2,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":"Demo","skills":[],"tokenUsage":null,"contextUsage":null}],"nextCursor":null,"backwardsCursor":null}""",
            )
            "thread/read" -> json.parseToJsonElement(
                """{"thread":{"id":"t1","sessionId":"s1","preview":"hello","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":2,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":"Demo","skills":[],"tokenUsage":null,"contextUsage":null,"turns":[]}}""",
            )
            "thread/resume" -> json.parseToJsonElement(
                """{"thread":{"id":"t1","sessionId":"s1","preview":"hello","ephemeral":false,"modelProvider":"openai","createdAt":1,"updatedAt":2,"lifecycleStatus":"active","path":null,"cwd":"/repo","cliVersion":"0","source":"appServer","threadSource":"user","agentNickname":null,"agentRole":null,"agentPath":"/root","gitInfo":null,"name":"Demo","skills":[],"tokenUsage":null,"contextUsage":null}}""",
            )
            else -> JsonObject(emptyMap())
        }
    }

    override fun notify(method: String, params: JsonElement?) = Unit

    override fun close() {
        closed = true
    }

    fun emitFailure() {
        events.tryEmit(RpcConnectionEvent.Failed(RpcConnectionException("network changed")))
    }

    suspend fun awaitEventSubscriber() {
        try {
            withTimeout(1_000) {
                while (events.subscriptionCount.value == 0) {
                    delay(1)
                }
            }
        } catch (error: TimeoutCancellationException) {
            assertTrue("No connection event subscriber", events.subscriptionCount.value > 0)
        }
    }

    private companion object {
        val json = Json { ignoreUnknownKeys = true }
    }
}
