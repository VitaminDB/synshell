"""Правка WayDroidService$2.onNotificationPosted (smali): заголовок и текст через getCharSequence
(у SpannableString getString даёт пустую строку), текст — android.bigText, иначе android.text;
сводки групп (FLAG_GROUP_SUMMARY) на хост не идут."""
import sys

p = sys.argv[1]
s = open(p).read()


def rep(a, b):
    global s
    assert s.count(a) == 1, a
    s = s.replace(a, b)


GET = "invoke-virtual {v3, %s, v7}, Landroid/os/Bundle;->getCharSequence(Ljava/lang/String;Ljava/lang/CharSequence;)Ljava/lang/CharSequence;\n\n    move-result-object %s\n\n    invoke-interface {%s}, Ljava/lang/CharSequence;->toString()Ljava/lang/String;\n\n    move-result-object %s\n"

rep("""    :cond_15
""", """    :cond_15
    iget v3, v2, Landroid/app/Notification;->flags:I

    and-int/lit16 v3, v3, 0x200

    if-eqz v3, :cond_syn_not_summary

    return-void

    :cond_syn_not_summary
""")
rep("""invoke-virtual {v3, v6, v7}, Landroid/os/Bundle;->getString(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;

    move-result-object v6
""", GET % ("v6", "v6", "v6", "v6"))
rep("""const-string v8, "android.text"

    invoke-virtual {v3, v8, v7}, Landroid/os/Bundle;->getString(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;

    move-result-object v8
""", 'const-string v8, "android.bigText"\n\n    ' + GET % ("v8", "v8", "v8", "v8") + """
    invoke-virtual {v8}, Ljava/lang/String;->isEmpty()Z

    move-result v10

    if-eqz v10, :cond_syn_text_ok

    const-string v8, "android.text"

    """ + GET % ("v8", "v8", "v8", "v8") + """
    :cond_syn_text_ok
""")
open(p, "w").write(s)
