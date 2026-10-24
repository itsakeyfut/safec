void *malloc(int n);
void free(void *p);

int main(void) {
    int *p = malloc(4);
    free(p);
#ifndef __x86_64__
    free(p);
#endif
    return 0;
}
