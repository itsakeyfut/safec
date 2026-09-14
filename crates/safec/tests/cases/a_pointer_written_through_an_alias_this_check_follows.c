void *malloc(int n);
void free(void *p);
int f(void) {
    int *p;
    int **pp = &p;
    *pp = malloc(4);
    *p = 1;
    return 0;
}
