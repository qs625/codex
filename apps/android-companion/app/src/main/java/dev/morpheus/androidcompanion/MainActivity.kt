package dev.morpheus.androidcompanion

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.enableEdgeToEdge
import androidx.activity.compose.setContent
import dev.morpheus.androidcompanion.ui.CompanionApp
import dev.morpheus.androidcompanion.ui.MorpheusTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            MorpheusTheme {
                CompanionApp()
            }
        }
    }
}
