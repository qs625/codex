package dev.morpheus.androidcompanion.state

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.morpheus.androidcompanion.model.toConversationThread
import dev.morpheus.androidcompanion.model.toThreadSummary
import dev.morpheus.androidcompanion.rpc.AppServerRpcClient
import dev.morpheus.androidcompanion.rpc.RpcError
import kotlinx.coroutines.Job
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject

class CompanionViewModel : ViewModel() {
    private val mutableState = MutableStateFlow(CompanionUiState())
    val state: StateFlow<CompanionUiState> = mutableState.asStateFlow()

    private var rpcClient: AppServerRpcClient? = null
    private var notificationJob: Job? = null

    fun connect(endpoint: String, token: String?) {
        val normalizedEndpoint = endpoint.trim()
        val endpointError = validateConnectionEndpoint(normalizedEndpoint)
        if (endpointError != null) {
            mutableState.update { it.copy(error = endpointError) }
            return
        }
        val normalizedToken = token?.trim()?.takeIf { it.isNotEmpty() }
        disconnect()
        mutableState.update {
            it.copy(connection = ConnectionState.Connecting, error = null)
        }
        viewModelScope.launch {
            try {
                val client = AppServerRpcClient(normalizedEndpoint, normalizedToken)
                client.connect()
                rpcClient = client
                notificationJob = launch {
                    client.serverNotifications.collect { notification ->
                        val projected = withContext(Dispatchers.Default) {
                            projectNotification(notification)
                        }
                        if (projected != null) {
                            mutableState.update { current ->
                                val (threads, selected) = applyProjectedNotification(
                                    current.threads,
                                    current.selectedThread,
                                    projected,
                                )
                                current.copy(threads = threads, selectedThread = selected)
                            }
                        }
                    }
                }
                mutableState.update {
                    it.copy(connection = ConnectionState.Connected(normalizedEndpoint))
                }
                refreshThreads()
            } catch (error: Throwable) {
                mutableState.update {
                    it.copy(
                        connection = ConnectionState.Failed(error.toUserMessage()),
                        error = error.toUserMessage(),
                    )
                }
            }
        }
    }

    fun disconnect() {
        notificationJob?.cancel()
        notificationJob = null
        rpcClient?.close()
        rpcClient = null
        mutableState.update { CompanionUiState() }
    }

    fun refreshThreads() {
        val client = rpcClient ?: return
        mutableState.update { it.copy(isLoadingThreads = true, error = null) }
        viewModelScope.launch {
            try {
                val result = client.request(
                    method = "thread/list",
                    params = buildJsonObject {
                        put("limit", JsonPrimitive(50))
                        put("sortKey", JsonPrimitive("updated_at"))
                        put("sortDirection", JsonPrimitive("desc"))
                    },
                )
                val threads = withContext(Dispatchers.Default) {
                    reduceThreadList(result)
                }
                val selectedId = mutableState.value.selectedThreadId ?: threads.firstOrNull()?.id
                mutableState.update {
                    it.copy(
                        threads = threads,
                        selectedThreadId = selectedId,
                        isLoadingThreads = false,
                    )
                }
                selectedId?.let { selectThread(it) }
            } catch (error: Throwable) {
                mutableState.update {
                    it.copy(isLoadingThreads = false, error = error.toUserMessage())
                }
            }
        }
    }

    fun selectThread(threadId: String) {
        val client = rpcClient ?: return
        mutableState.update {
            it.copy(selectedThreadId = threadId, isReadingThread = true, error = null)
        }
        viewModelScope.launch {
            try {
                val readResult = client.request(
                    method = "thread/read",
                    params = buildJsonObject {
                        put("threadId", JsonPrimitive(threadId))
                        put("includeTurns", JsonPrimitive(true))
                    },
                )
                val readThread = withContext(Dispatchers.Default) {
                    reduceThreadRead(readResult)
                }
                mutableState.update { applySelectedThreadRead(it, threadId, readThread) }
                val resumeResult = client.request(
                    method = "thread/resume",
                    params = buildJsonObject {
                        put("threadId", JsonPrimitive(threadId))
                        put("excludeTurns", JsonPrimitive(true))
                    },
                )
                val resumedSummary = withContext(Dispatchers.Default) {
                    resumeResult
                        .let { it as? JsonObject }
                        ?.get("thread")
                        ?.let { it as? JsonObject }
                        ?.toThreadSummary()
                }
                if (resumedSummary != null) {
                    mutableState.update { current ->
                        if (current.selectedThreadId == threadId) {
                            current.copy(threads = upsert(current.threads, resumedSummary))
                        } else {
                            current
                        }
                    }
                }
            } catch (error: Throwable) {
                mutableState.update {
                    if (it.selectedThreadId == threadId) {
                        it.copy(isReadingThread = false, error = error.toUserMessage())
                    } else {
                        it
                    }
                }
            }
        }
    }

    fun startThread(cwd: String?) {
        val client = rpcClient ?: return
        mutableState.update { it.copy(error = null) }
        viewModelScope.launch {
            try {
                val params = buildJsonObject {
                    cwd?.trim()?.takeIf { it.isNotEmpty() }?.let {
                        put("cwd", JsonPrimitive(it))
                    }
                    put("threadSource", JsonPrimitive("user"))
                }
                val result = client.request("thread/start", params)
                val thread = withContext(Dispatchers.Default) {
                    (result as? JsonObject)
                        ?.get("thread")
                        ?.let { it as? JsonObject }
                        ?.toConversationThread()
                }
                if (thread != null) {
                    mutableState.update {
                        it.copy(
                            threads = upsert(it.threads, thread.summary),
                            selectedThreadId = thread.id,
                            selectedThread = thread,
                        )
                    }
                }
            } catch (error: Throwable) {
                mutableState.update { it.copy(error = error.toUserMessage()) }
            }
        }
    }

    fun sendMessage(text: String) {
        val client = rpcClient ?: return
        val threadId = mutableState.value.selectedThreadId ?: return
        val trimmed = text.trim()
        if (trimmed.isEmpty()) return
        mutableState.update { it.copy(isSending = true, error = null) }
        viewModelScope.launch {
            try {
                client.request(
                    method = "turn/start",
                    params = buildJsonObject {
                        put("threadId", JsonPrimitive(threadId))
                        put(
                            "input",
                            buildJsonArray {
                                add(
                                    buildJsonObject {
                                        put("type", JsonPrimitive("text"))
                                        put("text", JsonPrimitive(trimmed))
                                    },
                                )
                            },
                        )
                    },
                )
                mutableState.update { it.copy(isSending = false) }
            } catch (error: Throwable) {
                mutableState.update {
                    it.copy(isSending = false, error = error.toUserMessage())
                }
            }
        }
    }

    override fun onCleared() {
        disconnect()
    }
}

private fun upsert(current: List<dev.morpheus.androidcompanion.model.ThreadSummary>, next: dev.morpheus.androidcompanion.model.ThreadSummary): List<dev.morpheus.androidcompanion.model.ThreadSummary> {
    return listOf(next) + current.filterNot { it.id == next.id }
}

private fun Throwable.toUserMessage(): String {
    return when (this) {
        is RpcError -> "Server error $code: $message"
        else -> message ?: "Unexpected error"
    }
}
