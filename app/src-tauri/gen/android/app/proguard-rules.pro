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
# The Rust side calls MainActivity.installApk through JNI, so nothing in Java refers to it and
# the release optimizer would remove it (the app then died with NoSuchMethodError on update).
-keep class dev.lexis.gre.MainActivity { public *; }
-keepclassmembers class dev.lexis.gre.MainActivity$Voice { @android.webkit.JavascriptInterface <methods>; }
