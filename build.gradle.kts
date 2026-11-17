// Pins the Gradle plugin classpath (AGP, Kotlin) in buildscript-gradle.lockfile
// (remediation §7); refresh with --write-locks like app/gradle.lockfile.
buildscript {
    configurations.classpath {
        resolutionStrategy.activateDependencyLocking()
    }
    // Raises vulnerable libraries that AGP 8.9.1 and Kotlin 2.0.21 pull onto
    // the build classpath to their first fixed releases (osv-scanner findings,
    // remediation §7). Build-time only: none of these ship in the APK.
    dependencies {
        classpath(platform("io.netty:netty-bom:4.1.137.Final"))
        constraints {
            classpath("org.bouncycastle:bcprov-jdk18on:1.85")
            classpath("org.bouncycastle:bcpkix-jdk18on:1.85")
            classpath("org.bouncycastle:bcutil-jdk18on:1.85")
            classpath("com.google.guava:guava:33.7.2-jre")
            classpath("com.google.protobuf:protobuf-java:3.25.5")
            classpath("org.apache.commons:commons-compress:1.26.0")
            classpath("org.apache.commons:commons-lang3:3.18.0")
            classpath("org.bitbucket.b_c:jose4j:0.9.6")
            classpath("org.jdom:jdom2:2.0.6.1")
        }
    }
}

plugins {
    id("com.android.application") version "8.9.1" apply false
    id("org.jetbrains.kotlin.android") version "2.0.21" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.0.21" apply false
    id("org.jetbrains.kotlin.plugin.serialization") version "2.0.21" apply false
}
