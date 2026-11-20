plugins {
  alias(libs.plugins.android.application)
  alias(libs.plugins.compose.compiler)
  alias(libs.plugins.kotlin.serialization)
}

android {
  namespace = "com.sonora.hiresplayer"
  compileSdk = 36

  defaultConfig {
    applicationId = "com.sonora.hiresplayer"
    minSdk = 26
    targetSdk = 36
    versionCode = 1
    versionName = "1.0.0"

    ndk {
      abiFilters += listOf("arm64-v8a")
    }
  }

  signingConfigs {
    create("sonoraRelease") {
      val ksFile = file("sonora-release.jks")
      if (ksFile.exists()) {
        storeFile = ksFile
        storePassword = "sonora-hires-audio"
        keyAlias = "sonora"
        keyPassword = "sonora-hires-audio"
      }
    }
  }

  buildTypes {
    release {
      isMinifyEnabled = false
      val ksFile = file("sonora-release.jks")
      signingConfig = if (ksFile.exists()) {
        signingConfigs.getByName("sonoraRelease")
      } else {
        signingConfigs.getByName("debug")
      }
      proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
    }
  }

  compileOptions {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
  }

  buildFeatures {
    compose = true
    aidl = false
    buildConfig = false
    shaders = false
  }

  packaging {
    resources {
      excludes += "/META-INF/{AL2.0,LGPL2.1}"
    }
  }
}

kotlin {
  jvmToolchain(17)
}

dependencies {
  val composeBom = platform(libs.androidx.compose.bom)
  implementation(composeBom)

  implementation(libs.androidx.core.ktx)
  implementation(libs.androidx.lifecycle.runtime.ktx)
  implementation(libs.androidx.activity.compose)
  implementation(libs.androidx.lifecycle.runtime.compose)
  implementation(libs.androidx.lifecycle.viewmodel.compose)

  implementation(libs.androidx.compose.ui)
  implementation(libs.androidx.compose.ui.tooling.preview)
  implementation(libs.androidx.compose.material3)
  debugImplementation(libs.androidx.compose.ui.tooling)

  testImplementation(libs.junit)
  testImplementation(libs.kotlinx.coroutines.test)
}
