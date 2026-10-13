package com.changjin.sidetask.mobile

import android.app.Activity
import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

@InvokeArg
class SessionRequest { var key: String = ""; var value: String? = null }

@TauriPlugin
class SecureStorePlugin(private val activity: Activity): Plugin(activity) {
    @Command
    fun timeZone(invoke: Invoke) {
        val zone = java.util.TimeZone.getDefault().id
        val normalized = if (zone in listOf("GMT", "GMT+00:00", "GMT-00:00", "Etc/UTC")) "UTC" else zone
        invoke.resolve(JSObject().put("value", normalized))
    }
    @Command
    fun systemCertificates(invoke: Invoke) {
        try {
            val store = KeyStore.getInstance("AndroidCAStore").apply { load(null) }
            val certificates = org.json.JSONArray()
            for (alias in store.aliases()) {
                if (alias.startsWith("system:")) {
                    certificates.put(Base64.encodeToString(store.getCertificate(alias).encoded, Base64.NO_WRAP))
                }
            }
            check(certificates.length() > 0)
            invoke.resolve(JSObject().put("certificates", certificates))
        } catch (_: Exception) { invoke.reject("无法读取系统信任证书") }
    }
    private val preferences get() = activity.getSharedPreferences("sidetask_sessions", Context.MODE_PRIVATE)
    private fun key(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey("sidetask-session-v1", null) as? SecretKey)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        generator.init(KeyGenParameterSpec.Builder("sidetask-session-v1", KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).build())
        return generator.generateKey()
    }
    private fun request(invoke: Invoke): SessionRequest {
        val args = invoke.parseArgs(SessionRequest::class.java)
        require(args.key.matches(Regex("[a-f0-9]{64}")))
        return args
    }
    @Command
    @Synchronized
    fun readSession(invoke: Invoke) {
        try {
            val args = request(invoke)
            val encoded = preferences.getString(args.key, null)
            val result = JSObject()
            if (encoded != null) {
                val bytes = Base64.decode(encoded, Base64.NO_WRAP)
                require(bytes.size in 29..65536)
                val cipher = Cipher.getInstance("AES/GCM/NoPadding")
                cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes.copyOfRange(0, 12)))
                cipher.updateAAD(args.key.toByteArray(Charsets.UTF_8))
                result.put("value", String(cipher.doFinal(bytes.copyOfRange(12, bytes.size)), Charsets.UTF_8))
            } else { result.put("value", org.json.JSONObject.NULL) }
            invoke.resolve(result)
        } catch (_: Exception) { invoke.reject("无法读取系统登录凭据") }
    }
    @Command
    @Synchronized
    fun writeSession(invoke: Invoke) {
        try {
            val args = request(invoke)
            val value = requireNotNull(args.value)
            require(value.toByteArray(Charsets.UTF_8).size <= 60000)
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.ENCRYPT_MODE, key())
            cipher.updateAAD(args.key.toByteArray(Charsets.UTF_8))
            val bytes = cipher.iv + cipher.doFinal(value.toByteArray(Charsets.UTF_8))
            check(preferences.edit().putString(args.key, Base64.encodeToString(bytes, Base64.NO_WRAP)).commit())
            invoke.resolve(JSObject().put("value", org.json.JSONObject.NULL))
        } catch (_: Exception) { invoke.reject("无法安全保存系统登录凭据") }
    }
    @Command
    @Synchronized
    fun deleteSession(invoke: Invoke) {
        try {
            val args = request(invoke)
            check(preferences.edit().remove(args.key).commit())
            invoke.resolve(JSObject().put("value", org.json.JSONObject.NULL))
        } catch (_: Exception) { invoke.reject("无法清理系统登录凭据") }
    }
}
