void g(int n) {
}
int returned(void) {
    char a[3];
    a[0] = 0;
    const char *p = a;
    return p == a;
}
int through_a_local(void) {
    const char *p = "ab";
    int r = p != 0;
    return r;
}
int main(void) {
    const char *p = "ab";
    g(p != 0);
    return returned() + through_a_local();
}
