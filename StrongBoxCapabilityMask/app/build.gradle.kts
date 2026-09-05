plugins {
    id("com.android.application")
}

android {
    namespace = "io.github.andrealyz.strongboxcapabilitymask"
    compileSdk = 36

    defaultConfig {
        applicationId = "io.github.andrealyz.strongboxcapabilitymask"
        minSdk = 29
        targetSdk = 36
        versionCode = 2
        versionName = "1.4.1"
        testInstrumentationRunner =
            "io.github.andrealyz.strongboxcapabilitymask.FeatureProbeInstrumentation"
    }

    signingConfigs {
        getByName("debug") {
            providers.gradleProperty("ciDebugKeystore").orNull?.let {
                storeFile = file(it)
            }
        }
    }

    buildTypes {
        release {
            signingConfig = signingConfigs.getByName("debug")
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    packaging {
        resources {
            merges += "META-INF/xposed/*"
        }
    }
}

dependencies {
    compileOnly("io.github.libxposed:api:102.0.0")
    testImplementation("junit:junit:4.13.2")
}
