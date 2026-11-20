int main(void) {
    char *s = "A\n\"q\\";
    char *t = "a" "b";
    return s[0] + t[1];
}
