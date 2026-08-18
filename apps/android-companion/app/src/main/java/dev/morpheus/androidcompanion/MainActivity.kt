package dev.morpheus.androidcompanion

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.enableEdgeToEdge
import androidx.activity.compose.setContent
import androidx.compose.runtime.remember
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.morpheus.androidcompanion.persistence.SharedPreferencesConnectionSettingsStore
import dev.morpheus.androidcompanion.state.CompanionViewModel
import dev.morpheus.androidcompanion.ui.CompanionApp
import dev.morpheus.androidcompanion.ui.MorpheusTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            MorpheusTheme {
                val settingsStore = remember {
                    SharedPreferencesConnectionSettingsStore(applicationContext)
                }
                val viewModel: CompanionViewModel = viewModel(
                    factory = CompanionViewModel.Factory(settingsStore),
                )
                CompanionApp(viewModel)
            }
        }
    }
}
