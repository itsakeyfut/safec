void *malloc(int n);
void free(void *p);
void opaque(int **pp);
int f(int c) {
    int *a = malloc(4);
    int *b = malloc(4);
    int *p;
    if (c) { p = a; } else { p = b; }
    free(p);
    int **pp = &p;
    opaque(pp);
    free(p);
    return 0;
}
