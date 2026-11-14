// `memchr`, `memcmp` and `strlen` read what they are handed and write none
// of it (C17 7.24.4, 7.24.5, 7.24.6.3), so an array nothing wrote is still
// unwritten after each, and a read of it is `SC0411` (#952). A function
// this check cannot read is believed to write what it is handed (#782),
// and `fill` is the control.
void *memchr(void *s, int c, int n);
int memcmp(void *a, void *b, int n);
int strlen(char *s);
void fill(int **p);

int searched(void) {
    int *a[1];
    memchr(a, 0, 8);
    int *q = a[0];
    return q != 0;
}

int compared(void) {
    int *a[1];
    int *b[1];
    b[0] = 0;
    memcmp(a, b, 8);
    int *q = a[0];
    return q != 0;
}

int measured(void) {
    char s[4];
    s[0] = 0;
    int *a[1];
    strlen(s);
    int *q = a[0];
    return q != 0;
}

int filled(void) {
    int *a[1];
    fill(a);
    int *q = a[0];
    return q != 0;
}
