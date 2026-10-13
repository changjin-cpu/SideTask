package com.changjin.sidetask

import android.os.Bundle
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback

class MainActivity : TauriActivity() {
  private var taskWebView: WebView? = null
  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    taskWebView = webView
    WebView.setWebContentsDebuggingEnabled(BuildConfig.DEBUG)
    onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
      override fun handleOnBackPressed() {
        webView.evaluateJavascript("Boolean(window.__sidetaskBack && window.__sidetaskBack())") { handled ->
          if (handled != "true") moveTaskToBack(true)
        }
      }
    })
  }
  override fun onPause() {
    taskWebView?.evaluateJavascript("window.__TAURI_INTERNALS__?.invoke('mobile_active', {active:false}).catch(()=>{})", null)
    super.onPause()
  }
  override fun onResume() {
    super.onResume()
    taskWebView?.evaluateJavascript("window.__TAURI_INTERNALS__?.invoke('mobile_active', {active:true}).catch(()=>{});window.dispatchEvent(new Event('focus'))", null)
  }
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
  }
}
