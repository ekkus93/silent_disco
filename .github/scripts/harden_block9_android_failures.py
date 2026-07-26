from pathlib import Path


def replace_once(path: Path, old: str, new: str, label: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    path.write_text(text.replace(old, new))


main_view_model = Path(
    "app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt"
)
replace_once(
    main_view_model,
    '''        runCatching {
            runBlocking(Dispatchers.IO) { domainStore.close() }
        }.onFailure { error ->
            logger.e("storage.close", error.message ?: "Failed to close Rust database", error)
        }
''',
    '''        runBlocking(Dispatchers.IO) { domainStore.close() }
''',
    "MainViewModel close failure visibility",
)

test_path = Path(
    "app/src/androidTest/java/com/ekkus/silentdisco/platform/persistence/"
    "AndroidRustDomainStoreInstrumentedTest.kt"
)
text = test_path.read_text()
anchor = '''    @Test
    fun corruptDatabaseFailureIsVisibleAndLeavesLegacyValuesIntact() {
'''
if text.count(anchor) != 1:
    raise SystemExit("corrupt database test anchor changed")
new_test = '''    @Test
    fun malformedLegacyTrustValueIsVisibleAndPreserved() {
        val suffix = System.nanoTime().toString()
        val preferencesName = "block9-malformed-trust-$suffix"
        val provider = AndroidDatabasePathProvider(
            context,
            "block9-malformed-trust-$suffix.sqlite3",
        )
        val preferences = context.getSharedPreferences(preferencesName, Context.MODE_PRIVATE)
        val malformedKey = LegacyPreferencesContract.trustedDeviceKey("listener-$suffix")
        preferences.edit()
            .putString(malformedKey, "not-a-boolean")
            .commit()
        val store = AndroidRustDomainStore(
            context = context,
            pathProvider = provider,
            preferencesName = preferencesName,
        )
        try {
            val error = assertThrows(AndroidRustDomainStoreException::class.java) {
                runBlocking { store.initialize() }
            }
            assertTrue(error.message.orEmpty().contains("does not contain a Boolean"))
            assertEquals("not-a-boolean", preferences.getString(malformedKey, null))
        } finally {
            runBlocking { store.close() }
            cleanup(preferencesName, provider.databasePath())
        }
    }

'''
test_path.write_text(text.replace(anchor, new_test + anchor))
