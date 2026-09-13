void *malloc(int n);
void free(void *p);
int f(int i) {
    int *p = malloc(4);
    free(p);
    for (; i; *p) { i = i - 1; }
    return 0;
}
