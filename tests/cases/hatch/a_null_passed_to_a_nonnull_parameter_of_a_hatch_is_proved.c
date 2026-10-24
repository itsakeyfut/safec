__attribute__((annotate("safec_unchecked")))
int raw(int * _Nonnull p) {
    return *p;
}

int main(void) {
    return raw(0);
}
