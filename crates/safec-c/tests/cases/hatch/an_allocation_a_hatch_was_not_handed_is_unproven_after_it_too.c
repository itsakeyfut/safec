void *malloc(int n);
void free(void *p);

__attribute__((annotate("safec_unchecked")))
void nothing(void) {
}

int main(void) {
    int *p = malloc(4);
    nothing();
    free(p);
    return 0;
}
