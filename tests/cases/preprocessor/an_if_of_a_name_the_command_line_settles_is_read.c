#if __GNUC__
#error the command line said the build does not define it
#endif
#if !defined(__GNUC__)
int main(void) { return 0; }
#endif
