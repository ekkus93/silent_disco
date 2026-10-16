package com.ekkus.silentdisco;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;

/** Test-only provider that can issue a real persistable URI grant to the target app. */
public final class PersistableGrantTestProvider extends ContentProvider {
    public static final String AUTHORITY = "com.ekkus.silentdisco.test.grants";
    public static final String GRANT_METHOD = "grant-persistable-read";
    public static final String URI_RESULT_KEY = "uri";

    @Override
    public boolean onCreate() {
        return true;
    }

    @Override
    public Bundle call(String method, String arg, Bundle extras) {
        if (!GRANT_METHOD.equals(method) || arg == null) {
            throw new IllegalArgumentException("Unexpected provider grant request");
        }
        String[] parts = arg.split("/", 2);
        if (parts.length != 2) throw new IllegalArgumentException("Expected package/token");
        Uri uri = Uri.parse("content://" + AUTHORITY + "/audio/" + parts[1]);
        getContext().grantUriPermission(
                parts[0],
                uri,
                android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION
                        | android.content.Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
        Bundle result = new Bundle();
        result.putString(URI_RESULT_KEY, uri.toString());
        return result;
    }

    @Override
    public Cursor query(Uri uri, String[] projection, String selection, String[] selectionArgs,
                        String sortOrder) {
        return null;
    }

    @Override
    public String getType(Uri uri) {
        return "audio/wav";
    }

    @Override
    public Uri insert(Uri uri, ContentValues values) {
        return null;
    }

    @Override
    public int delete(Uri uri, String selection, String[] selectionArgs) {
        return 0;
    }

    @Override
    public int update(Uri uri, ContentValues values, String selection, String[] selectionArgs) {
        return 0;
    }
}
