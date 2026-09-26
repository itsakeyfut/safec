__attribute__((annotate("safec_unchecked")))
int raw(int *p) {
    return *p;
}

int checked(int *p) {
    return *p;
}
