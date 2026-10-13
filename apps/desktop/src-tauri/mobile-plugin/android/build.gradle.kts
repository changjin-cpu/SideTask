plugins { id("com.android.library"); id("org.jetbrains.kotlin.android") }
android {
    namespace = "com.changjin.sidetask.mobile"
    compileSdk = 35
    defaultConfig { minSdk = 26 }
    compileOptions { sourceCompatibility = JavaVersion.VERSION_1_8; targetCompatibility = JavaVersion.VERSION_1_8 }
    kotlinOptions { jvmTarget = "1.8" }
}
dependencies { implementation(project(":tauri-android")) }
