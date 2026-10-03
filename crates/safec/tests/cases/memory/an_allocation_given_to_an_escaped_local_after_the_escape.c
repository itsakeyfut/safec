void *malloc(int n);
void free(void *p);
int f(void) {
    int *p;
    int **pp = &p;
    int *q = malloc(8);
    p = q;
    free(q);
    return 0;
}
