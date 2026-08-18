package dev.morpheus.androidcompanion.state

data class ConnectionSettings(
    val endpoint: String,
    val token: String?,
)

interface ConnectionSettingsStore {
    fun load(): ConnectionSettings?
    fun save(settings: ConnectionSettings)
}

object EmptyConnectionSettingsStore : ConnectionSettingsStore {
    override fun load(): ConnectionSettings? = null

    override fun save(settings: ConnectionSettings) = Unit
}
