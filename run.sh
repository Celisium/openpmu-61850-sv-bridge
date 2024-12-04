cargo build --manifest-path=sv_parse/Cargo.toml
# Cargo outputs the library as 'libsv_parse.so', but Python expects it to be named 'sv_parse.so'.
cp ./sv_parse/target/debug/libsv_parse.so ./sv_parse/target/debug/sv_parse.so
sudo capsh --keep=1 --user=$USER --inh=cap_net_raw --addamb=cap_net_raw -- -c "PYTHONPATH=./sv_parse/target/debug python -m mu_to_openpmu $@"
