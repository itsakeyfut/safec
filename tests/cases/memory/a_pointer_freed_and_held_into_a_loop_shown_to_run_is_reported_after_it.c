void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    if (!p) return 0;
    *p = 1;
    free(p);
    int *a = p;
    int *c = p;
    int *b = 0;
    for (int j = 0; j < 2; j++) { c = b; b = a; }
    if (c) return *c;
    return 0;
}
