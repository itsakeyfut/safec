void *malloc(int n);
void free(void *p);

__attribute__((annotate("safec_unchecked")))
void keep(int *p) {
}

int main(void) {
    int *p = malloc(4);
    keep(p);
    free(p);
    return 0;
}
