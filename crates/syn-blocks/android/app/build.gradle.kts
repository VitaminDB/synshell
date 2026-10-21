plugins {
    id("com.android.application")
}

android {
    namespace = "com.syngui.synblocks"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.syngui.synblocks"
        minSdk = 30
        targetSdk = 35
        versionCode = 1
        versionName = "0.2.0"
        // APK: syn-blocks-release.apk / syn-blocks-debug.apk
        setProperty("archivesBaseName", "syn-blocks")
        ndk {
            abiFilters += listOf("arm64-v8a")
        }
    }

    // Release подписывается отладочным ключом: сборка для своих устройств,
    // одна подпись позволяет обновлять установленную версию без удаления.
    signingConfigs {
        getByName("debug") {
            storeFile = file("${System.getProperty("user.home")}/.android/debug.keystore")
            storePassword = "android"
            keyAlias = "androiddebugkey"
            keyPassword = "android"
        }
    }

    buildTypes {
        getByName("release") {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    sourceSets {
        getByName("main") {
            jniLibs.srcDirs("src/main/jniLibs")
        }
    }
}

dependencies {
    // Версия обязана совпадать с GameActivity из android-activity (0.6.1 → 4.4.0).
    implementation("androidx.games:games-activity:4.4.0")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation(platform("org.jetbrains.kotlin:kotlin-bom:2.1.10"))
}
