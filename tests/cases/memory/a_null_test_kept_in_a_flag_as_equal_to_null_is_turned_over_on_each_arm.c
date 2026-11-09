void *malloc(int n);
int f(void) {
    int *b = malloc(4);
    int bad = b == 0;
    if (!bad) { *b = 1; return 0; } else { return *b; }
}
