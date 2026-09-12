void *malloc(int n);
void free(void *p);
int f(int i) {
    int *p = malloc(8);
    free(p);
    p[i] = 42;
    return 0;
}
