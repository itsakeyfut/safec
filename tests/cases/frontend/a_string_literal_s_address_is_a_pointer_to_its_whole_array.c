int main(void) {
    char (*whole)[3] = &"ab";
    char (*wrong)[9] = &"a";
    return (&"ab")[0][1];
}
