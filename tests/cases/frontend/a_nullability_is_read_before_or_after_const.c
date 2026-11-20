int g(int *const _Nonnull p) {
    return *p;
}
int h(int *_Nonnull const p) {
    return *p;
}
int main(void) {
    return 0;
}
