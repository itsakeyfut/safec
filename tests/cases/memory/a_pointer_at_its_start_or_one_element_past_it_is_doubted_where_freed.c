void *malloc(int n); void free(void *p);
void f(int c) {
    int *a = malloc(16);
    if (!a) return;
    int *q = a;
    if (c) q = a + 1;
    free(q);
}
