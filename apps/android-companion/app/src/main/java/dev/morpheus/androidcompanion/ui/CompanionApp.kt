package dev.morpheus.androidcompanion.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.CloudOff
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Divider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.morpheus.androidcompanion.model.ConversationItem
import dev.morpheus.androidcompanion.model.ToolPresentation
import dev.morpheus.androidcompanion.model.ThreadSummary
import dev.morpheus.androidcompanion.state.CompanionUiState
import dev.morpheus.androidcompanion.state.CompanionViewModel
import dev.morpheus.androidcompanion.state.ConnectionState
import dev.morpheus.androidcompanion.state.ConnectionPayloadParseResult
import dev.morpheus.androidcompanion.state.parseConnectionPayload
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions

private const val ExpandedTextLimit = 8_000

@Composable
fun CompanionApp(viewModel: CompanionViewModel = viewModel()) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    Scaffold { padding ->
        if (state.connection !is ConnectionState.Connected) {
            ConnectionScreen(
                state = state,
                modifier = Modifier.padding(padding),
                onConnect = viewModel::connect,
            )
        } else {
            WorkspaceScreen(
                state = state,
                modifier = Modifier.padding(padding),
                onRefresh = viewModel::refreshThreads,
                onSelectThread = viewModel::selectThread,
                onStartThread = viewModel::startThread,
                onSend = viewModel::sendMessage,
                onDisconnect = viewModel::disconnect,
            )
        }
    }
}

@Composable
private fun ConnectionScreen(
    state: CompanionUiState,
    modifier: Modifier,
    onConnect: (String, String?) -> Unit,
) {
    var endpoint by remember { mutableStateOf("ws://192.168.1.2:8910") }
    var token by remember { mutableStateOf("") }
    var scanError by remember { mutableStateOf<String?>(null) }
    val scanLauncher = rememberLauncherForActivityResult(ScanContract()) { result ->
        val contents = result.contents
        if (contents.isNullOrBlank()) {
            scanError = "No QR code scanned."
            return@rememberLauncherForActivityResult
        }
        when (val parsed = parseConnectionPayload(contents)) {
            is ConnectionPayloadParseResult.Success -> {
                endpoint = parsed.payload.endpoint
                token = parsed.payload.token.orEmpty()
                scanError = null
                onConnect(parsed.payload.endpoint, parsed.payload.token)
            }
            is ConnectionPayloadParseResult.Failure -> {
                scanError = parsed.message
            }
        }
    }
    Column(
        modifier = modifier
            .fillMaxSize()
            .padding(20.dp),
        verticalArrangement = Arrangement.Center,
    ) {
        Text("Morpheus", style = MaterialTheme.typography.headlineLarge, fontWeight = FontWeight.Bold)
        Text(
            "Connect to an app-server WebSocket listener.",
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(24.dp))
        OutlinedTextField(
            value = endpoint,
            onValueChange = { endpoint = it },
            label = { Text("WebSocket URL") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        Spacer(Modifier.height(12.dp))
        OutlinedTextField(
            value = token,
            onValueChange = { token = it },
            label = { Text("Bearer token") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        Spacer(Modifier.height(16.dp))
        Row(
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            modifier = Modifier.fillMaxWidth(),
        ) {
            OutlinedButton(
                onClick = {
                    scanLauncher.launch(
                        ScanOptions()
                            .setDesiredBarcodeFormats(ScanOptions.QR_CODE)
                            .setPrompt("Scan the Android Companion QR from Settings")
                            .setBeepEnabled(false),
                    )
                },
                enabled = state.connection != ConnectionState.Connecting,
                modifier = Modifier.weight(1f),
            ) {
                Text("Scan QR")
            }
            Button(
                onClick = {
                    scanError = null
                    onConnect(endpoint, token.takeIf { it.isNotBlank() })
                },
                enabled = state.connection != ConnectionState.Connecting,
                modifier = Modifier.weight(1f),
            ) {
                Text(if (state.connection == ConnectionState.Connecting) "Connecting" else "Connect")
            }
        }
        scanError?.let {
            Spacer(Modifier.height(12.dp))
            Text(it, color = MaterialTheme.colorScheme.error)
        }
        state.error?.let {
            Spacer(Modifier.height(12.dp))
            Text(it, color = MaterialTheme.colorScheme.error)
        }
    }
}

@Composable
private fun WorkspaceScreen(
    state: CompanionUiState,
    modifier: Modifier,
    onRefresh: () -> Unit,
    onSelectThread: (String) -> Unit,
    onStartThread: (String?) -> Unit,
    onSend: (String) -> Unit,
    onDisconnect: () -> Unit,
) {
    var tab by remember { mutableStateOf(0) }
    var cwd by remember { mutableStateOf("") }
    Scaffold(
        modifier = modifier.fillMaxSize(),
        bottomBar = {
            NavigationBar {
                NavigationBarItem(selected = tab == 0, onClick = { tab = 0 }, icon = {}, label = { Text("Threads") })
                NavigationBarItem(selected = tab == 1, onClick = { tab = 1 }, icon = {}, label = { Text("Conversation") })
            }
        },
    ) { inner ->
        Column(Modifier.padding(inner).fillMaxSize()) {
            Header(state, onRefresh, onDisconnect)
            state.error?.let {
                Text(
                    text = it,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                )
            }
            if (tab == 0) {
                ThreadPane(
                    state = state,
                    cwd = cwd,
                    onCwdChange = { cwd = it },
                    onSelectThread = {
                        onSelectThread(it)
                        tab = 1
                    },
                    onStartThread = {
                        onStartThread(cwd.takeIf { value -> value.isNotBlank() })
                        tab = 1
                    },
                )
            } else {
                ConversationPane(state = state, onSend = onSend)
            }
        }
    }
}

@Composable
private fun Header(
    state: CompanionUiState,
    onRefresh: () -> Unit,
    onDisconnect: () -> Unit,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            Modifier
                .size(10.dp)
                .clip(CircleShape)
                .background(Color(0xFF1B8F5A)),
        )
        Spacer(Modifier.width(8.dp))
        Text(
            text = (state.connection as? ConnectionState.Connected)?.endpoint ?: "Disconnected",
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f),
        )
        IconButton(onClick = onRefresh) {
            Icon(Icons.Default.Refresh, contentDescription = "Refresh")
        }
        IconButton(onClick = onDisconnect) {
            Icon(Icons.Default.CloudOff, contentDescription = "Disconnect")
        }
    }
    Divider()
}

@Composable
private fun ThreadPane(
    state: CompanionUiState,
    cwd: String,
    onCwdChange: (String) -> Unit,
    onSelectThread: (String) -> Unit,
    onStartThread: () -> Unit,
) {
    Column(Modifier.fillMaxSize()) {
        Row(
            Modifier
                .fillMaxWidth()
                .padding(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            OutlinedTextField(
                value = cwd,
                onValueChange = onCwdChange,
                label = { Text("CWD for new thread") },
                singleLine = true,
                modifier = Modifier.weight(1f),
            )
            Spacer(Modifier.width(8.dp))
            IconButton(onClick = onStartThread) {
                Icon(Icons.Default.Add, contentDescription = "New thread")
            }
        }
        Text(
            "Agent tree",
            style = MaterialTheme.typography.titleSmall,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
        )
        if (state.threads.isEmpty() && !state.isLoadingThreads) {
            EmptyMessage("No threads found.")
        } else {
            LazyColumn(contentPadding = PaddingValues(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                items(state.threads.agentTreeOrder(), key = { it.id }) { thread ->
                    ThreadRow(
                        thread = thread,
                        selected = thread.id == state.selectedThreadId,
                        onClick = { onSelectThread(thread.id) },
                    )
                }
            }
        }
    }
}

@Composable
private fun ThreadRow(thread: ThreadSummary, selected: Boolean, onClick: () -> Unit) {
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick),
        colors = CardDefaults.cardColors(
            containerColor = if (selected) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceVariant,
        ),
    ) {
        Row(Modifier.padding(12.dp), verticalAlignment = Alignment.Top) {
            Spacer(Modifier.width((thread.agentDepth() * 14).dp))
            Column(Modifier.weight(1f)) {
                Text(thread.title, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(thread.agentPath ?: thread.id, maxLines = 1, overflow = TextOverflow.Ellipsis, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(thread.lifecycleLabel, style = MaterialTheme.typography.labelMedium)
                    thread.agentRole?.let { Text(it, style = MaterialTheme.typography.labelMedium) }
                }
            }
        }
    }
}

private fun List<ThreadSummary>.agentTreeOrder(): List<ThreadSummary> {
    return sortedWith(compareBy<ThreadSummary> { it.agentPath ?: "/root/${it.updatedAt ?: 0}" }.thenByDescending { it.updatedAt ?: 0 })
}

private fun ThreadSummary.agentDepth(): Int {
    val path = agentPath ?: return 0
    return path.split('/').count { it.isNotBlank() }.minus(1).coerceIn(0, 4)
}

@Composable
private fun ConversationPane(state: CompanionUiState, onSend: (String) -> Unit) {
    var draft by remember(state.selectedThreadId) { mutableStateOf("") }
    var expandedToolItems by remember(state.selectedThreadId) { mutableStateOf(setOf<String>()) }
    val selectedThread = state.selectedThread
    val items = remember(selectedThread) {
        selectedThread?.turns.orEmpty().flatMap { turn -> turn.items }
    }
    Column(Modifier.fillMaxSize()) {
        Box(Modifier.weight(1f)) {
            if (state.isReadingThread) {
                EmptyMessage("Loading conversation.")
            } else if (state.selectedThread == null) {
                EmptyMessage("Select or create a thread.")
            } else {
                LazyColumn(
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(12.dp),
                    verticalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    items(items, key = { it.id }) { item ->
                        ConversationRow(
                            item = item,
                            expanded = item.id in expandedToolItems,
                            onToggleExpanded = {
                                expandedToolItems = if (item.id in expandedToolItems) {
                                    expandedToolItems - item.id
                                } else {
                                    expandedToolItems + item.id
                                }
                            },
                        )
                    }
                }
            }
        }
        Divider()
        Row(
            Modifier
                .fillMaxWidth()
                .padding(12.dp),
            verticalAlignment = Alignment.Bottom,
        ) {
            OutlinedTextField(
                value = draft,
                onValueChange = { draft = it },
                label = { Text("Message") },
                modifier = Modifier.weight(1f),
                minLines = 1,
                maxLines = 5,
            )
            Spacer(Modifier.width(8.dp))
            IconButton(
                enabled = state.canSend && draft.isNotBlank(),
                onClick = {
                    val text = draft
                    draft = ""
                    onSend(text)
                },
            ) {
                Icon(Icons.AutoMirrored.Filled.Send, contentDescription = "Send")
            }
        }
    }
}

@Composable
private fun ConversationRow(
    item: ConversationItem,
    expanded: Boolean,
    onToggleExpanded: () -> Unit,
) {
    val toolPresentation = item.toolPresentation
    if (toolPresentation != null) {
        ToolConversationRow(
            item = item,
            presentation = toolPresentation,
            expanded = expanded,
            onToggleExpanded = onToggleExpanded,
        )
        return
    }
    val isUser = item.type == "userMessage"
    Row(
        Modifier.fillMaxWidth(),
        horizontalArrangement = if (isUser) Arrangement.End else Arrangement.Start,
    ) {
        Surface(
            tonalElevation = 1.dp,
            shape = MaterialTheme.shapes.medium,
            color = if (isUser) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceVariant,
            modifier = Modifier.fillMaxWidth(if (isUser) 0.86f else 0.94f),
        ) {
            Column(Modifier.padding(12.dp)) {
                Text(item.title, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Spacer(Modifier.height(4.dp))
                Text(item.body.ifBlank { item.type }, style = MaterialTheme.typography.bodyMedium)
            }
        }
    }
}

@Composable
private fun ToolConversationRow(
    item: ConversationItem,
    presentation: ToolPresentation,
    expanded: Boolean,
    onToggleExpanded: () -> Unit,
) {
    Row(
        Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.Start,
    ) {
        Surface(
            tonalElevation = 1.dp,
            shape = MaterialTheme.shapes.medium,
            color = MaterialTheme.colorScheme.surfaceVariant,
            modifier = Modifier.fillMaxWidth(0.96f),
        ) {
            Column(
                Modifier
                    .clickable(onClick = onToggleExpanded)
                    .padding(12.dp),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(
                            item.title,
                            style = MaterialTheme.typography.labelLarge,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        Text(
                            presentation.summary,
                            style = MaterialTheme.typography.bodyMedium,
                            maxLines = if (expanded) Int.MAX_VALUE else 2,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    presentation.status?.let { status ->
                        StatusPill(status)
                    }
                    TextButton(onClick = onToggleExpanded) {
                        Text(if (expanded) "Hide" else "Details")
                    }
                }
                if (expanded) {
                    Spacer(Modifier.height(10.dp))
                    DetailBlock(
                        "Details",
                        presentation.details,
                        truncated = presentation.detailsIsTruncated,
                    )
                    if (presentation.outputLabel != null) {
                        Spacer(Modifier.height(10.dp))
                        DetailBlock(
                            presentation.outputLabel,
                            if (presentation.outputIsEmpty) "No output" else presentation.output.orEmpty(),
                            monospace = true,
                            truncated = presentation.outputIsTruncated,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun StatusPill(status: String) {
    Surface(
        shape = MaterialTheme.shapes.small,
        color = MaterialTheme.colorScheme.secondaryContainer,
    ) {
        Text(
            text = status,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSecondaryContainer,
            modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun DetailBlock(
    label: String,
    text: String,
    monospace: Boolean = false,
    truncated: Boolean = false,
) {
    val boundedText = remember(text) {
        if (text.length <= ExpandedTextLimit) {
            text
        } else if (monospace) {
            "[truncated ${text.length - ExpandedTextLimit} chars; showing latest output]\n" +
                text.takeLast(ExpandedTextLimit)
        } else {
            text.take(ExpandedTextLimit) + "\n[truncated]"
        }
    }
    Column {
        Text(
            label,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(3.dp))
        Text(
            boundedText,
            style = MaterialTheme.typography.bodySmall,
            fontFamily = if (monospace) FontFamily.Monospace else FontFamily.Default,
        )
        if (truncated || boundedText.length < text.length) {
            Spacer(Modifier.height(3.dp))
            Text(
                "Preview truncated for performance.",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun EmptyMessage(text: String) {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Text(text, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}
