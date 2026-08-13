package dev.morpheus.androidcompanion.rpc

import kotlinx.coroutines.async
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.WebSocketListener
import okio.ByteString
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit

class AppServerRpcClientTest {
    @Test
    fun requestCompletesWhenNotificationArrivesBeforeResponse() = runTest {
        val messages = LinkedBlockingQueue<String>()
        val server = MockWebServer()
        server.enqueue(
            MockResponse().withWebSocketUpgrade(
                object : WebSocketListener() {
                    override fun onMessage(webSocket: okhttp3.WebSocket, text: String) {
                        messages.add(text)
                        when {
                            text.contains("\"method\":\"initialize\"") -> {
                                webSocket.send("""{"id":1,"result":{"codexHome":"/tmp","userAgent":"test","platformFamily":"macos","platformOs":"macos"}}""")
                            }
                            text.contains("\"method\":\"thread/list\"") -> {
                                webSocket.send("""{"method":"thread/status/changed","params":{"threadId":"t1","lifecycleStatus":"active"}}""")
                                webSocket.send("""{"id":2,"result":{"data":[],"nextCursor":null,"backwardsCursor":null}}""")
                            }
                        }
                    }
                },
            ),
        )
        server.start()
        val client = AppServerRpcClient(server.url("/").toString().replace("http://", "ws://"), null)

        client.connect()
        val notification = async { client.serverNotifications.first() }
        val result = client.request("thread/list", JsonObject(emptyMap()))

        assertEquals("[]", result.jsonObject["data"].toString())
        assertEquals("thread/status/changed", notification.await().method)
        assertTrue(messages.poll(2, TimeUnit.SECONDS).orEmpty().contains("initialize"))
        client.close()
        server.shutdown()
    }

    @Test
    fun serverErrorCompletesMatchingRequestExceptionally() = runTest {
        val server = MockWebServer()
        server.enqueue(
            MockResponse().withWebSocketUpgrade(
                object : WebSocketListener() {
                    override fun onMessage(webSocket: okhttp3.WebSocket, text: String) {
                        when {
                            text.contains("\"method\":\"initialize\"") -> {
                                webSocket.send("""{"id":1,"result":{}}""")
                            }
                            text.contains("\"method\":\"thread/list\"") -> {
                                webSocket.send("""{"id":2,"error":{"code":-32603,"message":"boom"}}""")
                            }
                        }
                    }

                    override fun onMessage(webSocket: okhttp3.WebSocket, bytes: ByteString) = Unit
                },
            ),
        )
        server.start()
        val client = AppServerRpcClient(server.url("/").toString().replace("http://", "ws://"), null)
        client.connect()

        val error = runCatching { client.request("thread/list") }.exceptionOrNull()

        assertTrue(error is RpcError)
        assertEquals("boom", error?.message)
        client.close()
        server.shutdown()
    }
}
