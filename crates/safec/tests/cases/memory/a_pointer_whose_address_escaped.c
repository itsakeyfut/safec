void *malloc(int n);
void free(void *p);
int f(void) {
    int *a = malloc(4);
    int *p = malloc(4);
    int **pp = &p;
    *pp = a;
    free(a);
    free(p);
    return 0;
}
