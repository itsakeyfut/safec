void *memset(void *d, int c, unsigned long long n);
void w(char *s);
void stored(void) {
    char *s = "ab";
    s[0] = 120;
}
void filled(void) {
    char *s = "ab";
    memset(s, 0, 2);
}
void handed_to_a_writer(void) {
    w("ab");
}
