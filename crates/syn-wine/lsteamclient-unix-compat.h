/* synmobile: lsteamclient на upstream Wine (>= 10.x). В Wine Proton ntdll экспортирует wine_unix_to_nt_file_name /
 * wine_nt_to_unix_file_name; в upstream их нет — те же преобразования через ntdll_get_dos_file_name /
 * ntdll_get_unix_file_name из wine/unixlib.h. Семантика как в Proton (размер в WCHAR/байтах, STATUS_BUFFER_TOO_SMALL). */
#ifndef __LSTEAMCLIENT_UNIX_COMPAT_H
#define __LSTEAMCLIENT_UNIX_COMPAT_H

#include <string.h>
#include <stdlib.h>
#include "wine/unixlib.h"

static inline NTSTATUS wine_unix_to_nt_file_name( const char *name, WCHAR *buffer, ULONG *size )
{
    static const WCHAR prefix[] = {'\\','?','?','\\'};
    WCHAR *dos = NULL;
    NTSTATUS status;
    ULONG len, need;

    if (name[0] != '/') return STATUS_INVALID_PARAMETER;  /* relative paths are not supported */
    if ((status = ntdll_get_dos_file_name( name, &dos, FILE_OPEN_IF )) && status != STATUS_NO_SUCH_FILE)
        return status;
    if (!dos) return STATUS_NO_MEMORY;
    for (len = 0; dos[len]; len++) ;
    /* ntdll_get_dos_file_name даёт «C:\…» или «\\?\unix\…»; NT-имя — «\??\C:\…» */
    if (len >= 4 && dos[0] == '\\' && dos[1] == '\\' && dos[2] == '?' && dos[3] == '\\') need = len + 1;
    else need = len + 4 + 1;
    if (*size >= need && buffer)
    {
        if (need == len + 1) { memcpy( buffer, dos, len * sizeof(WCHAR) ); buffer[1] = '?'; buffer[2] = '?'; }
        else { memcpy( buffer, prefix, sizeof(prefix) ); memcpy( buffer + 4, dos, len * sizeof(WCHAR) ); }
        buffer[need - 1] = 0;
        status = STATUS_SUCCESS;
    }
    else status = STATUS_BUFFER_TOO_SMALL;
    *size = need;
    free( dos );
    return status;
}

static inline NTSTATUS wine_nt_to_unix_file_name( const OBJECT_ATTRIBUTES *attr, char *nameA, ULONG *size,
                                                  UINT disposition )
{
    const UNICODE_STRING *nt = attr->ObjectName;
    ULONG len = nt->Length / sizeof(WCHAR), off = 0;
    WCHAR *dos;
    char *unix_name = NULL;
    NTSTATUS status;

    if (len >= 4 && nt->Buffer[0] == '\\' && nt->Buffer[1] == '?' && nt->Buffer[2] == '?' && nt->Buffer[3] == '\\') off = 4;
    if (!(dos = (WCHAR *)malloc( (len - off + 1) * sizeof(WCHAR) ))) return STATUS_NO_MEMORY;
    memcpy( dos, nt->Buffer + off, (len - off) * sizeof(WCHAR) );
    dos[len - off] = 0;
    status = ntdll_get_unix_file_name( dos, &unix_name, disposition );
    free( dos );
    if (unix_name && (!status || status == STATUS_NO_SUCH_FILE))
    {
        ULONG need = strlen( unix_name ) + 1;
        if (*size >= need) memcpy( nameA, unix_name, need );
        else status = STATUS_BUFFER_TOO_SMALL;
        *size = need;
    }
    free( unix_name );
    return status;
}

#endif
