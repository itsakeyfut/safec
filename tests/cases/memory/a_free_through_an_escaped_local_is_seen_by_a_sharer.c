void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    int *q = p;
    int **pp = &p;
    free(p);
    free(q);
    return 0;
}
