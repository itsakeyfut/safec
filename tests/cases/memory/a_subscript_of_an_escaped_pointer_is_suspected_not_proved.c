void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(8);
    int **pp = &p;
    free(p);
    p[0] = 42;
    return 0;
}
