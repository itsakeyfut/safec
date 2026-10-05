void *malloc(int n);

__attribute__((annotate("safec_unchecked")))
int *make(void) {
    int *p = malloc(4);
    return p;
}

int main(void) {
    int *q = make();
    return *q;
}
