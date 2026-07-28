# Task 1 MainViewModel refactor failure

```text
 481 app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt
  92 app/src/main/java/com/ekkus/silentdisco/app/MainViewModelAudioPipeline.kt
 121 app/src/main/java/com/ekkus/silentdisco/app/MainViewModelDemo.kt
 110 app/src/main/java/com/ekkus/silentdisco/app/MainViewModelDiagnostics.kt
 533 app/src/main/java/com/ekkus/silentdisco/app/MainViewModelHostActions.kt
 224 app/src/main/java/com/ekkus/silentdisco/app/MainViewModelHostPlayback.kt
 277 app/src/main/java/com/ekkus/silentdisco/app/MainViewModelListenerActions.kt
 269 app/src/main/java/com/ekkus/silentdisco/app/MainViewModelListenerPlayback.kt
 105 app/src/main/java/com/ekkus/silentdisco/app/MainViewModelPersistence.kt
 101 app/src/main/java/com/ekkus/silentdisco/app/MainViewModelSupport.kt
 233 app/src/main/java/com/ekkus/silentdisco/app/MainViewModelSynchronization.kt
 478 app/src/main/java/com/ekkus/silentdisco/app/MainViewModelTransport.kt
Loading package information...                                                  Loading local repository...                                                     [                                       ] 3% Loading local repository...        [                                       ] 3% Fetch remote repository...         [=                                      ] 3% Fetch remote repository...         [=                                      ] 4% Fetch remote repository...         [=                                      ] 5% Fetch remote repository...         [==                                     ] 5% Fetch remote repository...         [==                                     ] 6% Fetch remote repository...         [==                                     ] 7% Fetch remote repository...         [==                                     ] 7% Computing updates...               [===                                    ] 8% Computing updates...               [===                                    ] 10% Computing updates...              [=======================================] 100% Computing updates...             

info: syncing channel updates for 1.97.1-x86_64-unknown-linux-gnu
info: latest update on 2026-07-16 for version 1.97.1 (8bab26f4f 2026-07-14)
info: downloading 5 components

  1.97.1-x86_64-unknown-linux-gnu installed - rustc 1.97.1 (8bab26f4f 2026-07-14)

info: using existing install for 1.97.1-x86_64-unknown-linux-gnu
info: default toolchain set to 1.97.1-x86_64-unknown-linux-gnu

  1.97.1-x86_64-unknown-linux-gnu unchanged - rustc 1.97.1 (8bab26f4f 2026-07-14)

    Updating crates.io index
error: cannot update the lock file /home/runner/work/silent_disco/silent_disco/rust/Cargo.lock because --locked was passed to prevent this
help: to generate the lock file without accessing the network, remove the --locked flag and use --offline instead.
FAILED_COMMAND=cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
EXIT_STATUS=101

```
