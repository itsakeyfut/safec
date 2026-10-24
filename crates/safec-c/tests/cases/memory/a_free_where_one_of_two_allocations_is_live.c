void *malloc(int n);
void free(void *p);
int f(int c) {
    int *a = malloc(4);
    int *b = malloc(4);
    int *p = a;
    free(a);
    if (c) { p = b; }
    free(p);
    return 0;
}
