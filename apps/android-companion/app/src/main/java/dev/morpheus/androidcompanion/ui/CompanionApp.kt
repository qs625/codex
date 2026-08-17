package dev.morpheus.androidcompanion.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.CloudOff
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.Button
import androidx.compose.material3.Divider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
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
import dev.morpheus.androidcompanion.model.ConversationCell
import dev.morpheus.androidcompanion.model.ConversationItem
import dev.morpheus.androidcompanion.model.ToolPresentation
import dev.morpheus.androidcompanion.model.ThreadSummary
import dev.morpheus.androidcompanion.model.buildConversationCells
import dev.morpheus.androidcompanion.state.CompanionUiState
import dev.morpheus.androidcompanion.state.CompanionViewModel
import dev.morpheus.androidcompanion.state.ConnectionState
import dev.morpheus.androidcompanion.state.ConnectionPayloadParseResult
import dev.morpheus.androidcompanion.state.parseConnectionPayload
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions

private const val ExpandedTextLimit = 8_000
private const val MessageBodyTextLimit = 8_000

internal enum class CompanionPage {
    Threads,
    Conversation,
}

internal fun resolveConnectedPage(
    requested: CompanionPage,
    selectedThreadId: String?,
): CompanionPage {
    return if (requested == CompanionPage.Conversation && selectedThreadId == null) {
        CompanionPage.Threads
    } else {
        requested
    }
}

internal fun connectedBackTarget(page: CompanionPage): CompanionPage? {
    return if (page == CompanionPage.Conversation) CompanionPage.Threads else null
}

@Composable
fun CompanionApp(viewModel: CompanionViewModel = viewModel()) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    var connectedPage by remember { mutableStateOf(CompanionPage.Threads) }
    val currentPage = resolveConnectedPage(connectedPage, state.selectedThreadId)
    LaunchedEffect(state.connection) {
        if (state.connection !is ConnectionState.Connected) {
            connectedPage = CompanionPage.Threads
        }
    }
    BackHandler(enabled = state.connection is ConnectionState.Connected && connectedBackTarget(currentPage) != null) {
        connectedBackTarget(currentPage)?.let { connectedPage = it }
    }
    Surface(
        modifier = Modifier.fillMaxSize(),
        color = MaterialTheme.colorScheme.background,
    ) {
        Box(
            modifier = Modifier
                .fillMaxSize()
                .background(MaterialTheme.colorScheme.background)
                .windowInsetsPadding(WindowInsets.safeDrawing),
        ) {
            if (state.connection !is ConnectionState.Connected) {
                ConnectionScreen(
                    state = state,
                    modifier = Modifier.fillMaxSize(),
                    onConnect = viewModel::connect,
                )
            } else {
                WorkspaceScreen(
                    state = state,
                    page = currentPage,
                    modifier = Modifier.fillMaxSize(),
                    onNavigateBack = {
                        connectedBackTarget(currentPage)?.let { connectedPage = it }
                    },
                    onRefresh = viewModel::refreshThreads,
                    onSelectThread = {
                        viewModel.selectThread(it)
                        connectedPage = CompanionPage.Conversation
                    },
                    onStartThread = {
                        viewModel.startThread(it)
                        connectedPage = CompanionPage.Conversation
                    },
                    onSend = viewModel::sendMessage,
                    onDisconnect = viewModel::disconnect,
                )
            }
        }
    }
}

@Composable
private fun AppChrome(
    title: String,
    subtitle: String?,
    showBack: Boolean,
    onBack: () -> Unit,
    onRefresh: (() -> Unit)?,
    onDisconnect: (() -> Unit)?,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .height(64.dp)
            .padding(horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (showBack) {
            IconButton(onClick = onBack) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
            }
        } else {
            Box(
                Modifier
                    .padding(horizontal = 8.dp)
                    .size(28.dp)
                    .clip(RoundedCornerShape(8.dp))
                    .background(MaterialTheme.colorScheme.primaryContainer),
                contentAlignment = Alignment.Center,
            ) {
                Text("M", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary)
            }
        }
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
            subtitle?.let {
                Text(it, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
        onRefresh?.let {
            IconButton(onClick = it) {
                Icon(Icons.Default.Refresh, contentDescription = "Refresh")
            }
        }
        onDisconnect?.let {
            IconButton(onClick = it) {
                Icon(Icons.Default.CloudOff, contentDescription = "Disconnect")
            }
        }
    }
    Divider(color = MaterialTheme.colorScheme.outline.copy(alpha = 0.45f))
}

@Composable
private fun ConnectionStatusLine(text: String, color: Color) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        Box(
            Modifier
                .size(8.dp)
                .clip(CircleShape)
                .background(color),
        )
        Text(
            text,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
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
    Column(modifier = modifier.fillMaxSize()) {
        AppChrome(
            title = "Morpheus",
            subtitle = "Android Companion",
            showBack = false,
            onBack = { },
            onRefresh = null,
            onDisconnect = null,
        )
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(20.dp),
            verticalArrangement = Arrangement.Center,
        ) {
            Text("Connect to Runtime", style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.SemiBold)
            Text(
                "Pair this device with your Morpheus runtime.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(20.dp))
            Surface(
                modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(8.dp),
                color = MaterialTheme.colorScheme.surfaceVariant,
                tonalElevation = 1.dp,
            ) {
                Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text("1. Scan QR code", style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold)
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
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text("Scan QR")
                    }
                    Text("2. Enter connection details", style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold)
                    OutlinedTextField(
                        value = endpoint,
                        onValueChange = { endpoint = it },
                        label = { Text("Endpoint") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = token,
                        onValueChange = { token = it },
                        label = { Text("Access token") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Button(
                        onClick = {
                            scanError = null
                            onConnect(endpoint, token.takeIf { it.isNotBlank() })
                        },
                        enabled = state.connection != ConnectionState.Connecting,
                        modifier = Modifier.fillMaxWidth(),
                        shape = RoundedCornerShape(8.dp),
                    ) {
                        Text(if (state.connection == ConnectionState.Connecting) "Connecting" else "Connect")
                    }
                }
            }
            Spacer(Modifier.height(16.dp))
            when (val connection = state.connection) {
                is ConnectionState.Failed -> ConnectionStatusLine("Connection failed", MaterialTheme.colorScheme.error)
                ConnectionState.Connecting -> ConnectionStatusLine("Connecting", MaterialTheme.colorScheme.tertiary)
                ConnectionState.Disconnected -> ConnectionStatusLine("Not connected", MaterialTheme.colorScheme.onSurfaceVariant)
                is ConnectionState.Connected -> ConnectionStatusLine("Connected to ${connection.endpoint}", MaterialTheme.colorScheme.primary)
            }
            scanError?.let {
                Spacer(Modifier.height(10.dp))
                Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
            }
            state.error?.let {
                Spacer(Modifier.height(10.dp))
                Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
            }
        }
    }
}

@Composable
private fun WorkspaceScreen(
    state: CompanionUiState,
    page: CompanionPage,
    modifier: Modifier,
    onNavigateBack: () -> Unit,
    onRefresh: () -> Unit,
    onSelectThread: (String) -> Unit,
    onStartThread: (String?) -> Unit,
    onSend: (String) -> Unit,
    onDisconnect: () -> Unit,
) {
    var cwd by remember { mutableStateOf("") }
    val selectedTitle = state.selectedThread?.summary?.title
        ?: state.threads.firstOrNull { it.id == state.selectedThreadId }?.title
    Column(modifier = modifier.fillMaxSize()) {
        AppChrome(
            title = if (page == CompanionPage.Conversation) selectedTitle ?: "Conversation" else "Morpheus",
            subtitle = if (page == CompanionPage.Conversation) {
                state.selectedThreadId ?: "Loading thread"
            } else {
                (state.connection as? ConnectionState.Connected)?.endpoint ?: "Connected"
            },
            showBack = page == CompanionPage.Conversation,
            onBack = onNavigateBack,
            onRefresh = if (page == CompanionPage.Threads) onRefresh else null,
            onDisconnect = if (page == CompanionPage.Threads) onDisconnect else null,
        )
        state.error?.let {
            Text(
                text = it,
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                style = MaterialTheme.typography.bodySmall,
            )
        }
        if (page == CompanionPage.Threads) {
            ThreadPane(
                state = state,
                cwd = cwd,
                onCwdChange = { cwd = it },
                onSelectThread = onSelectThread,
                onStartThread = {
                    onStartThread(cwd.takeIf { value -> value.isNotBlank() })
                },
            )
        } else {
            ConversationPane(
                state = state,
                modifier = Modifier.weight(1f),
                onSend = onSend,
            )
        }
    }
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
                .padding(horizontal = 16.dp, vertical = 12.dp),
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
        Row(
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("Threads", style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold)
            Spacer(Modifier.width(8.dp))
            StatusPill("${state.threads.size}")
        }
        if (state.threads.isEmpty() && !state.isLoadingThreads) {
            EmptyMessage("No threads found.")
        } else {
            LazyColumn(
                contentPadding = PaddingValues(start = 12.dp, top = 8.dp, end = 12.dp, bottom = 20.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
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
    Surface(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick),
        shape = RoundedCornerShape(8.dp),
        color = if (selected) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceVariant,
        tonalElevation = if (selected) 2.dp else 0.dp,
    ) {
        Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Spacer(Modifier.width((thread.agentDepth() * 14).dp))
            Box(
                Modifier
                    .size(34.dp)
                    .clip(RoundedCornerShape(8.dp))
                    .background(MaterialTheme.colorScheme.secondaryContainer),
                contentAlignment = Alignment.Center,
            ) {
                Text(thread.title.take(1).ifBlank { "T" }, style = MaterialTheme.typography.labelLarge)
            }
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(thread.title, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(thread.agentPath ?: thread.id, maxLines = 1, overflow = TextOverflow.Ellipsis, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(thread.lifecycleLabel, style = MaterialTheme.typography.labelMedium, color = statusColor(thread.lifecycleLabel))
                    thread.agentRole?.let { Text(it, style = MaterialTheme.typography.labelMedium) }
                }
            }
        }
    }
}

private fun statusColor(status: String): Color {
    return when (status.lowercase()) {
        "active", "running", "completed", "complete" -> Color(0xFF5ED37A)
        "waiting" -> Color(0xFFFFCA67)
        "failed", "errored", "error", "interrupted" -> Color(0xFFFF6B6B)
        else -> Color(0xFFAEB8C2)
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
private fun ConversationPane(
    state: CompanionUiState,
    modifier: Modifier = Modifier,
    onSend: (String) -> Unit,
) {
    var draft by remember(state.selectedThreadId) { mutableStateOf("") }
    var expandedToolItems by remember(state.selectedThreadId) { mutableStateOf(setOf<String>()) }
    var expandedMessageItems by remember(state.selectedThreadId) { mutableStateOf(setOf<String>()) }
    val selectedThread = state.selectedThread
    val cells = remember(selectedThread) {
        selectedThread?.turns.orEmpty().buildConversationCells()
    }
    val listState = rememberLazyListState()
    var followBottom by remember(state.selectedThreadId) { mutableStateOf(true) }
    val scrollTarget = remember(cells) { conversationScrollTarget(cells) }
    LaunchedEffect(listState, state.selectedThreadId) {
        snapshotFlow {
            listState.isScrollInProgress to isConversationNearBottom(listState)
        }.collect { (isScrolling, isNearBottom) ->
            if (isScrolling) {
                followBottom = isNearBottom
            } else if (isNearBottom) {
                followBottom = true
            }
        }
    }
    LaunchedEffect(scrollTarget) {
        if (followBottom && cells.isNotEmpty()) {
            listState.animateScrollToItem(cells.lastIndex)
        }
    }
    Column(modifier.fillMaxSize()) {
        Box(Modifier.weight(1f)) {
            if (state.isReadingThread) {
                EmptyMessage("Loading conversation.")
            } else if (state.selectedThread == null) {
                EmptyMessage("Select or create a thread.")
            } else {
                LazyColumn(
                    state = listState,
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(start = 12.dp, top = 12.dp, end = 12.dp, bottom = 16.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    items(
                        items = cells,
                        key = { it.id },
                        contentType = { it.kind },
                    ) { cell ->
                        ConversationCellRow(
                            cell = cell,
                            expandedToolItems = expandedToolItems,
                            expandedMessageItems = expandedMessageItems,
                            onToggleExpanded = {
                                expandedToolItems = if (it in expandedToolItems) {
                                    expandedToolItems - it
                                } else {
                                    expandedToolItems + it
                                }
                            },
                            onToggleMessageExpanded = {
                                expandedMessageItems = if (it in expandedMessageItems) {
                                    expandedMessageItems - it
                                } else {
                                    expandedMessageItems + it
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
                .imePadding()
                .padding(horizontal = 12.dp, vertical = 10.dp),
            verticalAlignment = Alignment.Bottom,
        ) {
            OutlinedTextField(
                value = draft,
                onValueChange = { draft = it },
                placeholder = { Text("Message Morpheus...") },
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

internal data class ConversationScrollTarget(
    val cellCount: Int,
    val lastCellId: String?,
    val lastContentSize: Int,
)

internal fun conversationScrollTarget(cells: List<ConversationCell>): ConversationScrollTarget {
    val lastCell = cells.lastOrNull()
    val contentSize = lastCell?.entries.orEmpty().sumOf { item ->
        item.body.length +
            item.toolPresentation?.details.orEmpty().length +
            item.toolPresentation?.output.orEmpty().length
    }
    return ConversationScrollTarget(
        cellCount = cells.size,
        lastCellId = lastCell?.id,
        lastContentSize = contentSize,
    )
}

internal fun isConversationNearBottom(
    lastVisibleIndex: Int?,
    totalItemsCount: Int,
    thresholdItems: Int = 1,
): Boolean {
    if (totalItemsCount <= 0) return true
    val lastVisible = lastVisibleIndex ?: return false
    return lastVisible >= totalItemsCount - 1 - thresholdItems
}

private fun isConversationNearBottom(listState: LazyListState): Boolean {
    val layoutInfo = listState.layoutInfo
    return isConversationNearBottom(
        lastVisibleIndex = layoutInfo.visibleItemsInfo.lastOrNull()?.index,
        totalItemsCount = layoutInfo.totalItemsCount,
    )
}

@Composable
private fun ConversationCellRow(
    cell: ConversationCell,
    expandedToolItems: Set<String>,
    expandedMessageItems: Set<String>,
    onToggleExpanded: (String) -> Unit,
    onToggleMessageExpanded: (String) -> Unit,
) {
    if (cell.kind == "tool") {
        ToolConversationCell(
            cell = cell,
            expandedToolItems = expandedToolItems,
            onToggleExpanded = onToggleExpanded,
        )
        return
    }
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        cell.entries.forEach { item ->
            ConversationRow(
                item = item,
                expanded = item.id in expandedMessageItems,
                onToggleExpanded = { onToggleMessageExpanded(item.id) },
            )
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
            shape = RoundedCornerShape(8.dp),
            color = if (isUser) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceVariant,
            modifier = Modifier.fillMaxWidth(if (isUser) 0.86f else 0.94f),
        ) {
            Column(Modifier.padding(12.dp)) {
                Text(item.title, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Spacer(Modifier.height(4.dp))
                BoundedMessageBody(
                    item.body.ifBlank { item.type },
                    contentIsTruncated = item.bodyIsTruncated,
                    expanded = expanded,
                    onToggleExpanded = onToggleExpanded,
                )
            }
        }
    }
}

@Composable
private fun BoundedMessageBody(
    text: String,
    contentIsTruncated: Boolean = false,
    expanded: Boolean = false,
    onToggleExpanded: () -> Unit = { },
) {
    val preview = remember(text) { messageBodyPreview(text) }
    Text(visibleMessageBodyText(text, expanded), style = MaterialTheme.typography.bodyMedium)
    if (preview.isTruncated || contentIsTruncated) {
        Spacer(Modifier.height(3.dp))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                when {
                    preview.isTruncated && !expanded -> "Showing preview only."
                    contentIsTruncated -> "Showing available body only."
                    else -> "Full body is shown."
                },
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.weight(1f),
            )
            if (preview.isTruncated) {
                TextButton(onClick = onToggleExpanded) {
                    Text(if (expanded) "Show Less" else "Show All")
                }
            }
        }
    }
}

internal data class MessageBodyPreview(
    val text: String,
    val isTruncated: Boolean,
)

internal fun messageBodyPreview(text: String): MessageBodyPreview {
    return if (text.length <= MessageBodyTextLimit) {
        MessageBodyPreview(text = text, isTruncated = false)
    } else {
        MessageBodyPreview(
            text = text.takeLast(MessageBodyTextLimit),
            isTruncated = true,
        )
    }
}

internal fun visibleMessageBodyText(text: String, expanded: Boolean): String {
    return if (expanded) text else messageBodyPreview(text).text
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
            shape = RoundedCornerShape(8.dp),
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
private fun ToolConversationCell(
    cell: ConversationCell,
    expandedToolItems: Set<String>,
    onToggleExpanded: (String) -> Unit,
) {
    Row(
        Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.Start,
    ) {
        Surface(
            tonalElevation = 1.dp,
            shape = RoundedCornerShape(8.dp),
            color = Color(0xFF161F28),
            modifier = Modifier.fillMaxWidth(0.96f),
        ) {
            Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                cell.entries.forEachIndexed { index, item ->
                    val presentation = item.toolPresentation ?: return@forEachIndexed
                    if (index > 0) {
                        Divider()
                    }
                    ToolEntryContent(
                        item = item,
                        presentation = presentation,
                        expanded = item.id in expandedToolItems,
                        onToggleExpanded = { onToggleExpanded(item.id) },
                    )
                }
            }
        }
    }
}

@Composable
private fun ToolEntryContent(
    item: ConversationItem,
    presentation: ToolPresentation,
    expanded: Boolean,
    onToggleExpanded: () -> Unit,
) {
    Column(Modifier.clickable(onClick = onToggleExpanded)) {
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

@Composable
private fun StatusPill(status: String) {
    Surface(
        shape = RoundedCornerShape(6.dp),
        color = statusColor(status).copy(alpha = 0.18f),
    ) {
        Text(
            text = status,
            style = MaterialTheme.typography.labelSmall,
            color = statusColor(status),
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
            text.takeLast(ExpandedTextLimit)
        } else {
            text.take(ExpandedTextLimit)
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
                if (monospace) "Showing latest portion only." else "Showing preview only.",
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
