void *malloc(int n);
void free(void *p);
int f(int c) {
    int *a = malloc(4);
    int *b = malloc(4);
    int *p;
    if (c) { p = a; } else { p = b; }
    free(p);
    *p = 1;
    return 0;
}
