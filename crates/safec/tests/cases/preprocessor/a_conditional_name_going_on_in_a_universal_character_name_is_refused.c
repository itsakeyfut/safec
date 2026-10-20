void *malloc(int n);
void free(void *p);
#define A 1
int main(void) {
    int *p = malloc(4);
    free(p);
#ifndef A\u00e9
    free(p);
#endif
    return 0;
}
