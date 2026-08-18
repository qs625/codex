package dev.morpheus.androidcompanion.state

import dev.morpheus.androidcompanion.model.ConversationThread
import dev.morpheus.androidcompanion.model.ThreadSummary

data class CompanionUiState(
    val connection: ConnectionState = ConnectionState.Disconnected,
    val connectionEndpoint: String = "",
    val connectionToken: String = "",
    val threads: List<ThreadSummary> = emptyList(),
    val selectedThreadId: String? = null,
    val selectedThread: ConversationThread? = null,
    val isLoadingThreads: Boolean = false,
    val isReadingThread: Boolean = false,
    val isSending: Boolean = false,
    val error: String? = null,
) {
    val canSend: Boolean
        get() = connection is ConnectionState.Connected &&
            selectedThreadId != null &&
            !isSending
}

sealed interface ConnectionState {
    data object Disconnected : ConnectionState
    data object Connecting : ConnectionState
    data class Reconnecting(val endpoint: String, val attempt: Int) : ConnectionState
    data class Connected(val endpoint: String) : ConnectionState
    data class Failed(val message: String) : ConnectionState
}
