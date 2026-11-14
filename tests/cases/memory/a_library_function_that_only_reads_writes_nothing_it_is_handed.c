// `memchr`, `memcmp` and `strlen` read what they are handed and write none
// of it (C17 7.24.4, 7.24.5, 7.24.6.3), so an array nothing wrote is still
// unwritten after each, and a read of it is `SC0411` (#952). A function
// this check cannot read is believed to write what it is handed (#782),
// and `fill` is the control. What a search returns points into what it
// searched, so a pointer `strchr` found is freed with it, and `*p` after
// `free(s)` is `SC0402`, a second free through it is `SC0401`, and a free
// of one found past the start is `SC0404`.
void *memchr(void *s, int c, int n);
int memcmp(void *a, void *b, int n);
int strlen(char *s);
void fill(int **p);
void *malloc(int n);
void free(void *p);
char *strchr(char *s, int c);

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

int found_then_freed(void) {
    char *s = malloc(8);
    if (!s) return 0;
    s[0] = 1;
    s[1] = 0;
    char *p = strchr(s, 1);
    if (!p) return 0;
    free(s);
    return *p;
}

void found_and_freed_twice(void) {
    char *s = malloc(8);
    if (!s) return;
    s[0] = 1;
    char *p = memchr(s, 1, 1);
    free(s);
    free(p);
}

void found_past_the_start_and_freed(void) {
    char *s = malloc(8);
    if (!s) return;
    s[0] = 0;
    s[1] = 1;
    char *p = memchr(s, 1, 2);
    free(p);
}
