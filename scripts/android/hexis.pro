# Keep rules for the release build (R8, full mode). Installed by scripts/android_patch_manifest.py.
# The Gradle template picks up every *.pro file under app/, so this file is part of every release build.
#
# Why this file exists (found on a real phone, 2026-11-24): the QR scanner crashed the whole app on its first camera
# frame in the release APK. ML Kit lists its component registrars by NAME in the manifest and creates them with
# reflection (`Class.forName(name).getDeclaredConstructor().newInstance()`). R8's full mode removes a constructor that
# no code calls, so the three registrar classes stayed in the APK without a constructor, ML Kit quietly registered no
# components, and the first frame hit a null inside ML Kit's logger (NullPointerException in `mlkit_vision_common`).
# Debug builds are not minified, which is why only the release APK failed.
-keep class * implements com.google.firebase.components.ComponentRegistrar { <init>(); }
-keep class com.google.mlkit.** { <init>(); }
-keep class com.google.firebase.components.** { *; }
-keepnames class com.google.mlkit.**
-keepnames class com.google.android.gms.internal.mlkit_**

# Android services that the system creates by name from the merged manifest (providers, services, receivers) must keep
# their constructors too; AAPT2 already generates keep rules for them, this is the belt for the braces.
-keep class * extends android.content.ContentProvider { <init>(); }
-keep class * extends android.app.Service { <init>(); }
