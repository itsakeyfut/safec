__attribute__((annotate("safec_unchecked")))
int raw(int *p) {
    return *p;
}

int main(void) {
    int x = 1;
    return raw(&x);
}
