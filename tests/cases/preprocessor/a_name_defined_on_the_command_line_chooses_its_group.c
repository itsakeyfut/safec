void *malloc(int n);
void free(void *p);

int main(void) {
    int *p = malloc(4);
    free(p);
#ifndef TWICE
    p = malloc(4);
#endif
#ifdef TWICE
    free(p);
#endif
    return 0;
}
