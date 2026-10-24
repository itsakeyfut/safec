void *malloc(int n);
void free(void *p);
int g(int x);
int f(void) {
    int *p = malloc(4);
    free(p);
    return g(*p);
}
