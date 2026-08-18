package dev.morpheus.androidcompanion.persistence

import android.content.Context
import dev.morpheus.androidcompanion.state.ConnectionSettings
import dev.morpheus.androidcompanion.state.ConnectionSettingsStore

class SharedPreferencesConnectionSettingsStore(context: Context) : ConnectionSettingsStore {
    private val preferences = context.applicationContext.getSharedPreferences(
        "connection_settings",
        Context.MODE_PRIVATE,
    )

    override fun load(): ConnectionSettings? {
        val endpoint = preferences.getString(KEY_ENDPOINT, null)
            ?.takeIf { it.isNotBlank() }
            ?: return null
        val token = preferences.getString(KEY_TOKEN, null)
            ?.takeIf { it.isNotBlank() }
        return ConnectionSettings(endpoint, token)
    }

    override fun save(settings: ConnectionSettings) {
        preferences.edit()
            .putString(KEY_ENDPOINT, settings.endpoint)
            .putString(KEY_TOKEN, settings.token.orEmpty())
            .apply()
    }

    private companion object {
        const val KEY_ENDPOINT = "endpoint"
        const val KEY_TOKEN = "token"
    }
}
