# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# If your project uses WebView with JS, uncomment the following
# and specify the fully qualified class name to the JavaScript interface
# class:
#-keepclassmembers class fqcn.of.javascript.interface.for.webview {
#   public *;
#}

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile
# Tauri finds the plugin, its commands and argument classes by reflection.
-keep class com.videoindiren.app.** { *; }
-keep class com.yausername.** { *; }
# youtubedl-android unzips its tools with commons-compress, which looks classes up by reflection.
-keep class org.apache.commons.** { *; }
-keep class com.fasterxml.jackson.** { *; }
-dontwarn org.apache.commons.**
-dontwarn com.fasterxml.jackson.**
