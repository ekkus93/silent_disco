import org.gradle.api.tasks.Delete
import org.gradle.api.tasks.Exec

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    id("org.jetbrains.kotlin.plugin.serialization")
}

val rustAndroidAbis = setOf(
    "armeabi-v7a",
    "arm64-v8a",
    "x86",
    "x86_64",
)
val rustGeneratedJniRoot = layout.buildDirectory.dir("generated/rustJniLibs")

android {
    namespace = "com.ekkus.silentdisco"
    compileSdk = 36
    ndkVersion = "28.2.13676358"

    defaultConfig {
        applicationId = "com.ekkus.silentdisco"
        minSdk = 29
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"

        ndk {
            abiFilters += rustAndroidAbis
        }

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        vectorDrawables {
            useSupportLibrary = true
        }
        externalNativeBuild {
            cmake {
                arguments("-DANDROID_STL=c++_shared")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
        create("pocDebug") {
            initWith(getByName("debug"))
            applicationIdSuffix = ".poc"
            versionNameSuffix = "-poc"
            matchingFallbacks += listOf("debug")
        }
    }

    sourceSets {
        getByName("debug").jniLibs.srcDir(rustGeneratedJniRoot.map { it.dir("debug") })
        getByName("pocDebug").jniLibs.srcDir(rustGeneratedJniRoot.map { it.dir("debug") })
        getByName("release").jniLibs.srcDir(rustGeneratedJniRoot.map { it.dir("release") })
        getByName("androidTest").assets.srcDir("src/test/resources")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
        freeCompilerArgs += listOf(
            "-opt-in=kotlinx.coroutines.ExperimentalCoroutinesApi",
        )
    }

    buildFeatures {
        compose = true
        buildConfig = true
        prefab = true
    }

    externalNativeBuild {
        cmake {
            path = file("src/main/cpp/CMakeLists.txt")
        }
    }

    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }

    testOptions {
        animationsDisabled = true
        managedDevices {
            localDevices {
                create("pixel2api29") {
                    device = "Pixel 2"
                    apiLevel = 29
                    systemImageSource = "aosp"
                    require64Bit = true
                }
            }
        }
    }
}

fun registerRustAndroidBuildTask(
    taskName: String,
    profile: String,
) = tasks.register<Exec>(taskName) {
    group = "rust"
    description = "Builds the Rust core for all supported Android ABIs using the $profile profile."

    val outputDirectory = rustGeneratedJniRoot.map { it.dir(profile) }
    inputs.file(rootProject.file("scripts/build-rust-android.sh"))
    inputs.files(rootProject.fileTree("rust") { exclude("target/**") })
    outputs.dir(outputDirectory)

    workingDir(rootProject.projectDir)
    commandLine(
        "bash",
        rootProject.file("scripts/build-rust-android.sh").absolutePath,
        profile,
        outputDirectory.get().asFile.absolutePath,
    )
}

val buildRustAndroidDebug = registerRustAndroidBuildTask(
    taskName = "buildRustAndroidDebug",
    profile = "debug",
)
val buildRustAndroidRelease = registerRustAndroidBuildTask(
    taskName = "buildRustAndroidRelease",
    profile = "release",
)

// Rust must be built before Android packages generated JNI libraries.
tasks.matching { it.name == "mergeDebugJniLibFolders" }.configureEach {
    dependsOn(buildRustAndroidDebug)
}
tasks.matching { it.name == "mergePocDebugJniLibFolders" }.configureEach {
    dependsOn(buildRustAndroidDebug)
}
tasks.matching { it.name == "mergeReleaseJniLibFolders" }.configureEach {
    dependsOn(buildRustAndroidRelease)
}

val cleanRustAndroid = tasks.register<Delete>("cleanRustAndroid") {
    group = "rust"
    description = "Deletes only generated Rust Android JNI libraries."
    delete(rustGeneratedJniRoot)
}
tasks.matching { it.name == "clean" }.configureEach {
    dependsOn(cleanRustAndroid)
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2024.09.03")

    implementation("androidx.core:core-ktx:1.18.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.10.0")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.10.0")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.10.0")
    implementation("androidx.navigation:navigation-compose:2.9.7")
    implementation("androidx.documentfile:documentfile:1.1.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3")
    implementation("com.google.oboe:oboe:1.10.0")

    implementation(composeBom)
    androidTestImplementation(composeBom)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")

    testImplementation("junit:junit:4.13.2")
    testImplementation("com.google.truth:truth:1.4.4")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")
    testImplementation("org.mockito:mockito-core:5.14.2")
    testImplementation("org.mockito.kotlin:mockito-kotlin:5.4.0")

    androidTestImplementation("androidx.test.ext:junit:1.3.0")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.7.0")
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
    androidTestImplementation("androidx.navigation:navigation-testing:2.9.7")
    androidTestImplementation("com.google.truth:truth:1.4.4")
    androidTestImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")

    debugImplementation("androidx.compose.ui:ui-tooling")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}
