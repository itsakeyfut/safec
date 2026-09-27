void *malloc(int n);
void free(void *p);

int *escaped(void) {
    int *p = malloc(4);
    int **pp = &p;
    free(p);
    return p;
}
