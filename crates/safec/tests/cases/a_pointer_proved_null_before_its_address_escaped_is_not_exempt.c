void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = 0;
    int **pp = &p;
    *pp = malloc(4);
    free(p);
    free(p);
    return 0;
}
